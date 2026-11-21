//! Post-processing of generated G-code: relative jobs, resume, other controllers, feeder, time estimate,
//! backlash and rotary A axis. Ported from LaserForge's Python engine.

use crate::gcode::JOB_HEADER;
use crate::model::MachineSettings;
use crate::util::{f0, f3};
use regex::{Captures, Regex};
use std::collections::HashMap;

fn partition(raw: &str) -> (&str, &str, &str) {
    match raw.find(';') {
        Some(i) => (&raw[..i], ";", &raw[i + 1..]),
        None => (raw, "", ""),
    }
}

/// "Current position" jobs become relative (G91); "User origin" jobs get G90 + G0 to the origin, then G91.
pub fn to_relative(gcode: &str, start: (f64, f64), user_origin: bool) -> String {
    let word = Regex::new(r"(?i)([XYZ])\s*(-?\d*\.?\d+)").unwrap();
    let g91 = Regex::new(r"(?i)G91\s*").unwrap();
    let g90 = Regex::new(r"(?i)G90\s*").unwrap();
    let mut pos: HashMap<char, f64> = HashMap::from([('X', start.0), ('Y', start.1), ('Z', 0.0)]);
    let mut out: Vec<String> = Vec::new();
    let mut rel_in = false;
    let mut header_done = false;
    for raw in gcode.lines() {
        let (head0, sep, tail) = partition(raw);
        let code = head0.to_uppercase();
        if !header_done {
            out.push(raw.to_string());
            if code.trim().starts_with(JOB_HEADER) {
                if user_origin {
                    out.push("G90".into());
                    out.push(format!("G0 X{} Y{}", f3(start.0), f3(start.1)));
                }
                out.push("G91".into());
                header_done = true;
            }
            continue;
        }
        if code.trim_start().starts_with('$') {
            out.push(raw.to_string());
            continue;
        }
        let mut head = head0.to_string();
        if code.contains("G91") {
            rel_in = true;
            head = g91.replace_all(&head, "").into_owned();
        }
        if code.contains("G90") {
            rel_in = false;
            head = g90.replace_all(&head, "").into_owned();
        }
        let new_head = if rel_in {
            for c in word.captures_iter(&head) {
                let ax = c[1].to_uppercase().chars().next().unwrap();
                *pos.get_mut(&ax).unwrap() += c[2].parse::<f64>().unwrap_or(0.0);
            }
            head
        } else {
            word.replace_all(&head, |c: &Captures| {
                let ax = c[1].to_uppercase().chars().next().unwrap();
                let v: f64 = c[2].parse().unwrap_or(0.0);
                let d = v - pos[&ax];
                pos.insert(ax, v);
                format!("{}{}", ax, f3(d))
            })
            .into_owned()
        };
        if !new_head.trim().is_empty() || !tail.trim().is_empty() {
            let mut s = new_head.trim_end().to_string();
            if !new_head.trim().is_empty() && !sep.is_empty() {
                s.push(' ');
            }
            s.push_str(sep);
            s.push_str(tail);
            out.push(s.trim().to_string());
        }
    }
    if header_done {
        let idx = (0..out.len()).rev().find(|&k| out[k].trim().to_uppercase().starts_with("M2")).unwrap_or(out.len());
        out.insert(idx, "G90".into());
    }
    out.join("\n")
}

fn word_re() -> Regex {
    Regex::new(r"([A-Z])\s*(-?\d*\.?\d+)").unwrap()
}

/// "Start from here": replays the modal state reached before `line_index` and continues from there.
pub fn resume_from(gcode: &str, line_index: usize) -> String {
    let lines: Vec<&str> = gcode.split('\n').collect();
    let line_index = line_index.min(lines.len());
    let word = word_re();
    let (mut units, mut dist) = ("G21".to_string(), "G90".to_string());
    let mut laser: Option<String> = None;
    let mut air = "M9".to_string();
    let (mut feed, mut x, mut y, mut z): (Option<f64>, f64, f64, Option<f64>) = (None, 0.0, 0.0, None);
    for raw in &lines[..line_index] {
        let ln = raw.split(';').next().unwrap_or("").trim().to_uppercase();
        if ln.is_empty() {
            continue;
        }
        for code in ["G20", "G21"] {
            if ln.starts_with(code) {
                units = code.into();
            }
        }
        for code in ["G90", "G91"] {
            if ln.contains(code) {
                dist = code.into();
            }
        }
        if ln.starts_with("M3") || ln.starts_with("M4") {
            laser = Some(ln[..2].to_string());
        }
        if ln.starts_with("M5") {
            laser = None;
        }
        if ln.starts_with("M7") || ln.starts_with("M8") || ln.starts_with("M9") {
            air = ln[..2].to_string();
        }
        for c in word.captures_iter(&ln) {
            let k = &c[1];
            let v: f64 = c[2].parse().unwrap_or(0.0);
            let rel = dist == "G91" && !ln.starts_with('$');
            match k {
                "X" => x = if rel { x + v } else { v },
                "Y" => y = if rel { y + v } else { v },
                "Z" => z = Some(if rel { z.unwrap_or(0.0) + v } else { v }),
                "F" => feed = Some(v),
                _ => {}
            }
        }
    }
    let mut out = vec!["; === LaserForge: resumed job ===".to_string(), format!("; continues from line {}", line_index + 1), units, "G90".into()];
    if air != "M9" {
        out.push(air);
    }
    if let Some(zv) = z {
        out.push(format!("G0 Z{}", f3(zv)));
    }
    out.push(format!("G0 X{} Y{} S0", f3(x), f3(y)));
    if let Some(l) = &laser {
        out.push(format!("{l} S0"));
    }
    if dist == "G91" {
        out.push("G91".into());
    }
    if let (Some(f), Some(_)) = (feed.filter(|f| *f != 0.0), &laser) {
        out.push(format!("G1 F{}", f0(f)));
    }
    out.push("; --- original job continues ---".into());
    out.extend(lines[line_index..].iter().map(|s| s.to_string()));
    out.join("\n")
}

/// Rewrite the GRBL-style output for another G-code controller.
pub fn translate_flavor(gcode: &str, m: &MachineSettings) -> String {
    let fl = m.flavor.as_str();
    if matches!(fl, "grbl" | "grblhal" | "fluidnc") {
        return gcode.to_string();
    }
    let smax = m.max_power.max(1) as f64;
    let s_word = Regex::new(r"S(\d+(?:\.\d+)?)").unwrap();
    let g4 = Regex::new(r"^G4\s+P([\d.]+)").unwrap();
    let m4 = Regex::new(r"^M4\b").unwrap();
    let mut out: Vec<String> = Vec::new();
    for line in gcode.split('\n') {
        let (code, sep, comment0) = partition(line);
        let mut comment = comment0.to_string();
        let mut c = code.trim().to_string();
        if !c.is_empty() {
            if c.starts_with("$H") {
                c = match fl { "marlin" => "G28 X Y", "smoothie" => "G28.2 X0 Y0", _ => "" }.to_string();
                if c.is_empty() {
                    comment = format!("{} homing is done from the LinuxCNC interface", comment).trim().to_string();
                }
            }
            if let Some(mm) = g4.captures(&c.clone()) {
                if fl == "marlin" || fl == "smoothie" {
                    let ms = (mm[1].parse::<f64>().unwrap_or(0.0) * 1000.0).round_ties_even() as i64;
                    c = format!("G4 P{ms}");
                }
            }
            if fl == "marlin" {
                c = s_word.replace_all(&c, |mm: &Captures| format!("S{}", (mm[1].parse::<f64>().unwrap_or(0.0) / smax * 255.0).round_ties_even() as i64)).into_owned();
                if m.marlin_air_fan {
                    c = match c.as_str() { "M7" | "M8" => "M106 S255".into(), "M9" => "M107".into(), _ => c };
                }
                if c == "M2" {
                    c = "M5".into();
                }
            } else if fl == "smoothie" {
                c = s_word.replace_all(&c, |mm: &Captures| format!("S{}", f3(mm[1].parse::<f64>().unwrap_or(0.0) / smax))).into_owned();
                c = m4.replace(&c, "M3").into_owned();
            } else if fl == "linuxcnc" {
                c = m4.replace(&c, "M3").into_owned();
            }
        }
        let mut s = c.clone();
        if !sep.is_empty() {
            if !c.is_empty() {
                s.push(' ');
            }
            s.push_str(sep);
            s.push_str(&comment);
        }
        out.push(s);
    }
    let text = out.iter().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
    if fl == "linuxcnc" {
        format!("%\n{text}\n%")
    } else {
        text
    }
}

/// Repeat the job, advancing the material between runs.
pub fn apply_feeder(gcode: &str, m: &MachineSettings) -> String {
    if !m.feeder_enabled || m.feeder_repeats < 1 {
        return gcode.to_string();
    }
    let lines: Vec<&str> = gcode.split('\n').collect();
    let tail_start = lines.iter().position(|l| l.starts_with("; --- job end ---")).unwrap_or_else(|| {
        lines.iter().position(|l| l.starts_with("; === Job Complete") || l.starts_with("G0 X0 Y0 ; return")).unwrap_or(lines.len())
    });
    let (body, tail) = lines.split_at(tail_start);
    let feed: Vec<String> = if m.feeder_axis == "custom" {
        if m.feeder_gcode.trim().is_empty() { vec![] } else { vec![m.feeder_gcode.trim().to_string()] }
    } else {
        let a = &m.feeder_axis;
        let mut f = vec!["M5".into(), "G91".into(), format!("G1 {}{} F{}", a, f3(m.feeder_distance), f0(m.feeder_speed)), "G90".into()];
        if a != "X" && a != "Y" {
            f.push(format!("G92 {a}0"));
        }
        f
    };
    let mut out: Vec<String> = Vec::new();
    for n in 0..m.feeder_repeats {
        out.push(format!("; --- feeder run {}/{} ---", n + 1, m.feeder_repeats));
        out.extend(body.iter().map(|s| s.to_string()));
        if n < m.feeder_repeats - 1 {
            out.push("; advance material".into());
            out.extend(feed.iter().cloned());
        }
    }
    out.extend(tail.iter().map(|s| s.to_string()));
    out.join("\n")
}

enum Move {
    Dwell(f64),
    Go { g0: bool, x: f64, y: f64, feed: Option<f64> },
}

/// Letter + number words of a G-code line (same as the Python regex `([A-Z])\s*(-?\d*\.?\d+)`), last one wins.
fn words(ln: &str) -> [Option<f64>; 26] {
    let b = ln.as_bytes();
    let mut w = [None; 26];
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_uppercase() {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let start = j;
            if j < b.len() && b[j] == b'-' {
                j += 1;
            }
            let ds = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j < b.len() && b[j] == b'.' && j + 1 < b.len() && b[j + 1].is_ascii_digit() {
                j += 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
            }
            if j > ds {
                w[(c - b'A') as usize] = ln[start..j].parse().ok();
                i = j;
                continue;
            }
        }
        i += 1;
    }
    w
}

fn moves(gcode: &str) -> Vec<Move> {
    let (mut x, mut y, mut feed, mut rel, mut g0) = (0.0, 0.0, None::<f64>, false, true);
    let mut out = Vec::new();
    let idx = |c: u8| (c - b'A') as usize;
    for raw in gcode.lines() {
        let ln = raw.split(';').next().unwrap_or("").trim().to_uppercase();
        if ln.is_empty() || ln.starts_with('$') {
            continue;
        }
        if ln.contains("G91") {
            rel = true;
        }
        if ln.contains("G90") {
            rel = false;
        }
        let w = words(&ln);
        if let Some(gi) = w[idx(b'G')] {
            if gi == 0.0 || gi == 1.0 || gi == 2.0 || gi == 3.0 {
                g0 = gi == 0.0;
            } else if gi == 4.0 {
                out.push(Move::Dwell(w[idx(b'P')].unwrap_or(0.0)));
                continue;
            }
        }
        if let Some(f) = w[idx(b'F')] {
            feed = Some(f);
        }
        let (gx, gy) = (w[idx(b'X')], w[idx(b'Y')]);
        if gx.is_none() && gy.is_none() {
            continue;
        }
        let nx = if rel { x + gx.unwrap_or(0.0) } else { gx.unwrap_or(x) };
        let ny = if rel { y + gy.unwrap_or(0.0) } else { gy.unwrap_or(y) };
        out.push(Move::Go { g0, x: nx, y: ny, feed });
        x = nx;
        y = ny;
    }
    out
}

/// Job time in seconds: trapezoidal acceleration and junction slowdowns, like GRBL's planner.
pub fn estimate_gcode_time(gcode: &str, m: &MachineSettings) -> f64 {
    let acc = if m.acceleration != 0.0 { m.acceleration } else { 1000.0 }.max(1.0);
    let rapid = (m.max_speed / 60.0).max(1.0);
    let mut segs: Vec<(f64, f64, f64, f64)> = Vec::new();
    let mut dwell = 0.0;
    let (mut px, mut py) = (0.0, 0.0);
    for mv in moves(gcode) {
        match mv {
            Move::Dwell(d) => dwell += d,
            Move::Go { g0, x, y, feed } => {
                let (dx, dy) = (x - px, y - py);
                let l = dx.hypot(dy);
                px = x;
                py = y;
                if l < 1e-6 {
                    continue;
                }
                let v = match feed {
                    Some(f) if !g0 && f != 0.0 => rapid.min(f / 60.0),
                    _ => rapid,
                };
                segs.push((l, v, dx / l, dy / l));
            }
        }
    }
    let n = segs.len();
    if n == 0 {
        return dwell;
    }
    let mut vj = vec![0.0f64; n + 1];
    for i in 1..n {
        let c = segs[i - 1].2 * segs[i].2 + segs[i - 1].3 * segs[i].3;
        vj[i] = segs[i - 1].1.min(segs[i].1) * c.max(0.0).powi(2);
    }
    for i in 0..n {
        vj[i + 1] = vj[i + 1].min((vj[i].powi(2) + 2.0 * acc * segs[i].0).sqrt());
    }
    for i in (0..n).rev() {
        vj[i] = vj[i].min((vj[i + 1].powi(2) + 2.0 * acc * segs[i].0).sqrt());
    }
    let mut t = dwell;
    for (i, &(l, v, _, _)) in segs.iter().enumerate() {
        let (v0, v1) = (vj[i], vj[i + 1]);
        let d_acc = ((v * v - v0 * v0) / (2.0 * acc)).max(0.0);
        let d_dec = ((v * v - v1 * v1) / (2.0 * acc)).max(0.0);
        if d_acc + d_dec <= l {
            t += (v - v0) / acc + (v - v1) / acc + (l - d_acc - d_dec) / v;
        } else {
            let vp = ((2.0 * acc * l + v0 * v0 + v1 * v1) / 2.0).max(0.0).sqrt();
            t += (vp - v0).max(0.0) / acc + (vp - v1).max(0.0) / acc;
        }
    }
    t
}

/// Backlash compensation on reversals, for absolute moves and relative (G91) sections.
pub fn apply_backlash(gcode: &str, bx: f64, by: f64) -> String {
    if bx <= 0.0 && by <= 0.0 {
        return gcode.to_string();
    }
    let word = Regex::new(r"([XY])(-?\d*\.?\d+)").unwrap();
    let motion = Regex::new(r"^\s*(G9[01]\s*)?(G0?[0123]\b|[XY])").unwrap();
    let mut rel = false;
    let mut last: [Option<f64>; 2] = [None, None];
    let mut dirn = [1i32, 1i32];
    let comp = [bx, by];
    let mut out: Vec<String> = Vec::new();
    for raw in gcode.lines() {
        let code = raw.split(';').next().unwrap_or("").to_uppercase();
        if code.contains("G91") {
            rel = true;
        }
        if code.contains("G90") {
            rel = false;
        }
        if code.trim_start().starts_with('$') || !word.is_match(&code) || !motion.is_match(&code) {
            out.push(raw.to_string());
            continue;
        }
        let (head, sep, tail) = partition(raw);
        let new = word.replace_all(head, |c: &Captures| {
            let ax = if &c[1] == "X" { 0 } else { 1 };
            let v: f64 = c[2].parse().unwrap_or(0.0);
            if rel {
                let mut extra = 0.0;
                if v.abs() > 1e-6 {
                    let nd = if v > 0.0 { 1 } else { -1 };
                    if nd != dirn[ax] {
                        extra = comp[ax] * nd as f64;
                        dirn[ax] = nd;
                    }
                }
                if let Some(l) = last[ax].as_mut() {
                    *l += v;
                }
                format!("{}{}", &c[1], f3(v + extra))
            } else {
                if let Some(prev) = last[ax] {
                    if (v - prev).abs() > 1e-6 {
                        dirn[ax] = if v > prev { 1 } else { -1 };
                    }
                }
                last[ax] = Some(v);
                format!("{}{}", &c[1], f3(v - if dirn[ax] < 0 { comp[ax] } else { 0.0 }))
            }
        });
        out.push(format!("{}{}{}", new, sep, tail));
    }
    out.join("\n")
}

/// Rotary on a real A axis: the job's Y travel is sent as A.
pub fn apply_rotary_axis(gcode: &str, m: &MachineSettings) -> String {
    if !(m.rotary_enabled && m.rotary_axis == "A") {
        return gcode.to_string();
    }
    let motion = Regex::new(r"(?i)^\s*(G0?[0123]\b|\$J=)").unwrap();
    let yw = Regex::new(r"(^|[^A-Z])Y(-?\d*\.?\d+)").unwrap();
    gcode
        .lines()
        .map(|ln| {
            let (head, sep, tail) = partition(ln);
            let h = if motion.is_match(head) { yw.replace_all(head, "${1}A${2}").into_owned() } else { head.to_string() };
            format!("{h}{sep}{tail}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
