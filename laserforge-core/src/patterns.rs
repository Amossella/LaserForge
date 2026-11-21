//! Test patterns: material test (power × speed grid), interval test and focus test. Ported from
//! laserforge/engine.py (material_test, interval_test, focus_test).

use crate::gcode::Generator;
use crate::model::{AirAssist, MachineSettings};
use crate::post::translate_flavor;
use serde_json::Value;

fn num(d: &Value, k: &str, def: f64) -> f64 {
    d.get(k).and_then(|v| v.as_f64()).unwrap_or(def)
}

fn air(d: &Value) -> AirAssist {
    if d.get("air_assist").and_then(|v| v.as_str()).unwrap_or("on") == "off" { AirAssist::Off } else { AirAssist::On }
}

use crate::util::py_float as pyf;

fn square_fill(g: &mut Generator, x: f64, y: f64, size: f64, interval: f64, speed: f64, power: f64) {
    let (mut yy, mut n) = (y, 0);
    while yy <= y + size + 1e-9 {
        let (a, b) = if n % 2 == 0 { (x, x + size) } else { (x + size, x) };
        g.rapid(a, yy);
        g.cut(b, yy, speed, power);
        yy += interval;
        n += 1;
    }
}

pub fn material_test(m: &MachineSettings, d: &Value) -> String {
    let rows = (num(d, "rows", 5.0) as i64).clamp(1, 20);
    let cols = (num(d, "cols", 5.0) as i64).clamp(1, 20);
    let (size, gap) = (num(d, "size", 10.0), num(d, "gap", 3.0));
    let (x0, y0) = (num(d, "x", 10.0), num(d, "y", 10.0));
    let (p_min, p_max) = (num(d, "power_min", 10.0), num(d, "power_max", 100.0));
    let (s_min, s_max) = (num(d, "speed_min", 500.0), num(d, "speed_max", 3000.0));
    let fill = d.get("mode").and_then(|v| v.as_str()).unwrap_or("fill") == "fill";
    let interval = num(d, "interval", 0.2);
    let mut g = Generator::new(m);
    g.lines.clear();
    g.add(format!("; Material test {rows}x{cols}: power {}-{}% (rows), speed {}-{} mm/min (cols)", pyf(p_min), pyf(p_max), pyf(s_min), pyf(s_max)));
    g.add("G21");
    g.add("G90");
    g.add("M4");
    g.set_air(air(d));
    for r in 0..rows {
        let power = p_min + (p_max - p_min) * if rows > 1 { r as f64 / (rows - 1) as f64 } else { 1.0 };
        for c in 0..cols {
            let speed = s_min + (s_max - s_min) * if cols > 1 { c as f64 / (cols - 1) as f64 } else { 1.0 };
            let (sx, sy) = (x0 + c as f64 * (size + gap), y0 + r as f64 * (size + gap));
            g.add(format!("; cell r{r} c{c}: {power:.0}% @ {speed:.0}"));
            if fill {
                square_fill(&mut g, sx, sy, size, interval, speed, power);
            } else {
                g.rapid(sx, sy);
                for (px, py) in [(sx + size, sy), (sx + size, sy + size), (sx, sy + size), (sx, sy)] {
                    g.cut(px, py, speed, power);
                }
            }
        }
    }
    g.add("S0");
    g.add("M5");
    g.set_air(AirAssist::Off);
    translate_flavor(&g.lines.join("\n"), m)
}

pub fn interval_test(m: &MachineSettings, d: &Value) -> String {
    let count = (num(d, "count", 6.0) as i64).clamp(2, 20);
    let (i_min, i_max) = (num(d, "interval_min", 0.05), num(d, "interval_max", 0.3));
    let (size, gap) = (num(d, "size", 10.0), num(d, "gap", 3.0));
    let (speed, power) = (num(d, "speed", 3000.0), num(d, "power", 40.0));
    let (x0, y0) = (num(d, "x", 10.0), num(d, "y", 10.0));
    let mut g = Generator::new(m);
    g.lines.clear();
    g.add(format!("; Interval test: {count} cells {}-{} mm @ {} mm/min {}%", pyf(i_min), pyf(i_max), pyf(speed), pyf(power)));
    g.add("G21");
    g.add("G90");
    g.add("M4 S0");
    g.set_air(air(d));
    for i in 0..count {
        let interval = i_min + (i_max - i_min) * i as f64 / (count - 1) as f64;
        g.add(format!("; cell {i}: interval {interval:.3}"));
        square_fill(&mut g, x0 + i as f64 * (size + gap), y0, size, interval, speed, power);
    }
    g.add("S0");
    g.add("M5");
    g.set_air(AirAssist::Off);
    translate_flavor(&g.lines.join("\n"), m)
}

pub fn focus_test(m: &MachineSettings, d: &Value) -> String {
    let count = (num(d, "count", 11.0) as i64).clamp(2, 30);
    let (z_start, z_step) = (num(d, "z_start", -5.0), num(d, "z_step", 1.0));
    let (length, gap) = (num(d, "length", 30.0), num(d, "gap", 4.0));
    let (speed, power) = (num(d, "speed", 1000.0), num(d, "power", 30.0));
    let (x0, y0) = (num(d, "x", 10.0), num(d, "y", 10.0));
    let mut g = Generator::new(m);
    g.lines.clear();
    g.add(format!("; Focus test: {count} lines, Z {} step {}", pyf(z_start), pyf(z_step)));
    g.add("G21");
    g.add("G90");
    g.add("M3 S0");
    g.set_air(air(d));
    for i in 0..count {
        let z = z_start + z_step * i as f64;
        let y = y0 + i as f64 * gap;
        g.add(format!("; line {i}: Z {z:.2}"));
        g.set_z(z);
        g.rapid(x0, y);
        g.cut(x0 + length, y, speed, power);
    }
    g.add("S0");
    g.set_z(0.0);
    g.add("M5");
    g.set_air(AirAssist::Off);
    translate_flavor(&g.lines.join("\n"), m)
}
