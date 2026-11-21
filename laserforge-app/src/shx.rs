//! Single-line (engraving) fonts: AutoCAD SHX files and the built-in Hershey fonts.
//! SHX ported from laserforge/shx.py; Hershey rendering follows the Hershey-Fonts package (MIT, see
//! assets/hershey-LICENSE.txt) that the Python app uses, with its font data embedded.

use crate::shapes::{Shape, P};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub const HERSHEY_PREFIX: &str = "Hershey:";
pub const SHX_PREFIX: &str = "SHX:";

/// Settings › SHX font folder.
pub static SHX_FOLDER: Mutex<String> = Mutex::new(String::new());

// ====================================================================== SHX

const DIRS: [(f64, f64); 16] = [(1.0, 0.0), (1.0, 0.5), (1.0, 1.0), (0.5, 1.0), (0.0, 1.0), (-0.5, 1.0), (-1.0, 1.0), (-1.0, 0.5),
                                (-1.0, 0.0), (-1.0, -0.5), (-1.0, -1.0), (-0.5, -1.0), (0.0, -1.0), (0.5, -1.0), (1.0, -1.0), (1.0, -0.5)];

fn s8(b: u8) -> f64 {
    b as i8 as f64
}

pub struct ShxFont {
    unifont: bool,
    shapes: HashMap<u32, Vec<u8>>,
    above: f64,
}

fn u16_at(d: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*d.get(p)?, *d.get(p + 1)?]))
}

impl ShxFont {
    pub fn parse(data: &[u8]) -> Result<ShxFont, String> {
        let head_end = data.iter().position(|&b| b == 0x1a).ok_or("non è un file SHX")? + 1;
        let header = String::from_utf8_lossy(&data[..head_end]).to_lowercase();
        let unifont = header.contains("unifont");
        if header.contains("bigfont") {
            return Err("SHX bigfont (font asiatici) non supportati".into());
        }
        let mut shapes = HashMap::new();
        let (mut above, mut _below) = (1u8, 0u8);
        let mut pos = head_end;
        if unifont {
            let count = u32::from_le_bytes(data.get(pos..pos + 4).ok_or("SHX troncato")?.try_into().unwrap());
            let info_len = u16_at(data, pos + 4).ok_or("SHX troncato")? as usize;
            pos += 6;
            let info = data.get(pos..pos + info_len).ok_or("SHX troncato")?;
            pos += info_len;
            if let Some(ne) = info.iter().position(|&b| b == 0) {
                if info.len() > ne + 2 {
                    above = info[ne + 1];
                    _below = info[ne + 2];
                }
            }
            for _ in 0..count.saturating_sub(1) {
                if pos + 4 > data.len() {
                    break;
                }
                let (num, ln) = (u16_at(data, pos).unwrap() as u32, u16_at(data, pos + 2).unwrap() as usize);
                pos += 4;
                let body = &data[pos.min(data.len())..(pos + ln).min(data.len())];
                pos += ln;
                let spec = match body.iter().position(|&b| b == 0) { Some(nz) => &body[nz + 1..], None => body };
                shapes.insert(num, spec.to_vec());
            }
        } else {
            let count = u16_at(data, pos + 4).ok_or("SHX troncato")? as usize;
            pos += 6;
            let index: Vec<(u32, usize)> = (0..count).filter_map(|i| Some((u16_at(data, pos + 4 * i)? as u32, u16_at(data, pos + 4 * i + 2)? as usize))).collect();
            pos += 4 * count;
            for (num, ln) in index {
                let body = &data[pos.min(data.len())..(pos + ln).min(data.len())];
                pos += ln;
                let nz = body.iter().position(|&b| b == 0);
                let spec = match nz { Some(nz) => &body[nz + 1..], None => body };
                if num == 0 {
                    if spec.len() >= 2 {
                        above = spec[0];
                        _below = spec[1];
                    }
                } else {
                    shapes.insert(num, spec.to_vec());
                }
            }
        }
        Ok(ShxFont { unifont, shapes, above: if above == 0 { 1.0 } else { above as f64 } })
    }

    fn cmd_len(&self, spec: &[u8], i: usize) -> usize {
        let pair_end = |step: usize| {
            let mut j = i + 1;
            while j + 1 < spec.len() && !(spec[j] == 0 && spec[j + 1] == 0) {
                j += step;
            }
            j + 2 - i
        };
        match spec[i] {
            3 | 4 => 2,
            7 => if self.unifont { 3 } else { 2 },
            8 | 10 => 3,
            9 => pair_end(2),
            11 => 6,
            12 => 4,
            13 => pair_end(3),
            _ => 1,
        }
    }

    /// (strokes, advance) in font units for one character.
    pub fn glyph(&self, code: u32, depth: usize) -> (Vec<Vec<P>>, f64) {
        let Some(spec) = self.shapes.get(&code).filter(|_| depth <= 8) else { return (Vec::new(), self.above * 0.6) };
        struct St {
            strokes: Vec<Vec<P>>,
            cur: Option<usize>,
            x: f64,
            y: f64,
            pen: bool,
        }
        let mut st = St { strokes: Vec::new(), cur: None, x: 0.0, y: 0.0, pen: true };
        fn mv(st: &mut St, dx: f64, dy: f64) {
            let (nx, ny) = (st.x + dx, st.y + dy);
            if st.pen {
                if st.cur.is_none() {
                    st.strokes.push(vec![[st.x, st.y]]);
                    st.cur = Some(st.strokes.len() - 1);
                }
                st.strokes[st.cur.unwrap()].push([nx, ny]);
            } else {
                st.cur = None;
            }
            st.x = nx;
            st.y = ny;
        }
        fn arc(st: &mut St, cx: f64, cy: f64, r: f64, a0: f64, a1: f64) {
            let n = 4.max(((a1 - a0).abs() / (std::f64::consts::PI / 16.0)) as i64);
            for k in 1..=n {
                let a = a0 + (a1 - a0) * k as f64 / n as f64;
                let (dx, dy) = (cx + r * a.cos() - st.x, cy + r * a.sin() - st.y);
                mv(st, dx, dy);
            }
        }
        fn bulge(st: &mut St, dx: f64, dy: f64, b: f64) {
            if b == 0.0 {
                mv(st, dx, dy);
                return;
            }
            let chord = dx.hypot(dy);
            let theta = 4.0 * (b / 127.0).atan();
            let r = chord / (2.0 * (theta / 2.0).sin());
            let (mx, my) = (st.x + dx / 2.0, st.y + dy / 2.0);
            let d = r * (theta / 2.0).cos();
            let (nx, ny) = (-dy / chord, dx / chord);
            let (cx, cy) = (mx + nx * d, my + ny * d);
            let a0 = (st.y - cy).atan2(st.x - cx);
            arc(st, cx, cy, r.abs(), a0, a0 + theta);
        }
        let mut scale = 1.0;
        let mut stack: Vec<(f64, f64)> = Vec::new();
        let at = |i: usize| spec.get(i).copied().unwrap_or(0);
        let mut i = 0;
        while i < spec.len() {
            let c = spec[i];
            match c {
                0 => break,
                1 => {
                    st.pen = true;
                    i += 1;
                }
                2 => {
                    st.pen = false;
                    st.cur = None;
                    i += 1;
                }
                3 => {
                    scale /= at(i + 1).max(1) as f64;
                    i += 2;
                }
                4 => {
                    scale *= at(i + 1) as f64;
                    i += 2;
                }
                5 => {
                    stack.push((st.x, st.y));
                    i += 1;
                }
                6 => {
                    if let Some((x, y)) = stack.pop() {
                        st.x = x;
                        st.y = y;
                    }
                    st.cur = None;
                    i += 1;
                }
                7 => {
                    let sub = if self.unifont {
                        i += 3;
                        ((at(i - 2) as u32) << 8) | at(i - 1) as u32
                    } else {
                        i += 2;
                        at(i - 1) as u32
                    };
                    let (ss, sadv) = self.glyph(sub, depth + 1);
                    for s in ss {
                        st.strokes.push(s.iter().map(|p| [st.x + p[0] * scale, st.y + p[1] * scale]).collect());
                    }
                    st.x += sadv * scale;
                    st.cur = None;
                }
                8 => {
                    mv(&mut st, s8(at(i + 1)) * scale, s8(at(i + 2)) * scale);
                    i += 3;
                }
                9 => {
                    i += 1;
                    while i + 1 < spec.len() && !(spec[i] == 0 && spec[i + 1] == 0) {
                        mv(&mut st, s8(spec[i]) * scale, s8(spec[i + 1]) * scale);
                        i += 2;
                    }
                    i += 2;
                }
                10 => {
                    let r = at(i + 1) as f64 * scale;
                    let sc = at(i + 2) as i8;
                    let cw = sc < 0;
                    let sc = sc.unsigned_abs();
                    let (start, count) = ((sc >> 4) & 7, if sc & 7 == 0 { 8 } else { sc & 7 });
                    let a0 = start as f64 * std::f64::consts::PI / 4.0;
                    let (cx, cy) = (st.x - r * a0.cos(), st.y - r * a0.sin());
                    let span = count as f64 * std::f64::consts::PI / 4.0;
                    arc(&mut st, cx, cy, r, a0, if cw { a0 - span } else { a0 + span });
                    i += 3;
                }
                11 => {
                    let (so, eo, rh, rl) = (at(i + 1) as f64, at(i + 2) as f64, at(i + 3) as f64, at(i + 4) as f64);
                    let r = (rh * 256.0 + rl) * scale;
                    let sc = at(i + 5) as i8;
                    let cw = sc < 0;
                    let sc = sc.unsigned_abs();
                    let (start, count) = ((sc >> 4) & 7, if sc & 7 == 0 { 8 } else { sc & 7 });
                    let q = std::f64::consts::PI / 4.0;
                    let a0 = start as f64 * q + (so / 256.0) * q * if cw { -1.0 } else { 1.0 };
                    let span = (count as f64 - 1.0) * q + (if eo != 0.0 { eo / 256.0 } else { 1.0 }) * q - (so / 256.0) * q;
                    let (cx, cy) = (st.x - r * a0.cos(), st.y - r * a0.sin());
                    arc(&mut st, cx, cy, r, a0, if cw { a0 - span } else { a0 + span });
                    i += 6;
                }
                12 => {
                    bulge(&mut st, s8(at(i + 1)) * scale, s8(at(i + 2)) * scale, s8(at(i + 3)));
                    i += 4;
                }
                13 => {
                    i += 1;
                    while i + 1 < spec.len() && !(spec[i] == 0 && spec[i + 1] == 0) {
                        bulge(&mut st, s8(spec[i]) * scale, s8(spec[i + 1]) * scale, s8(at(i + 2)));
                        i += 3;
                    }
                    i += 2;
                }
                14 => {
                    i += 1;
                    if i < spec.len() {
                        i += self.cmd_len(spec, i);
                    }
                }
                _ => {
                    let (len, dir) = ((c >> 4) as f64, (c & 15) as usize);
                    let (dx, dy) = DIRS[dir];
                    mv(&mut st, dx * len * scale, dy * len * scale);
                    i += 1;
                }
            }
        }
        (st.strokes.into_iter().filter(|s| s.len() >= 2).collect(), st.x)
    }

    /// Open strokes in mm, baseline at y = 0, Y up; height = cap height.
    pub fn text_strokes(&self, text: &str, height: f64) -> (Vec<Vec<P>>, f64) {
        let k = height / self.above;
        let (mut out, mut pen) = (Vec::new(), 0.0);
        for ch in text.chars() {
            let (strokes, adv) = self.glyph(ch as u32, 0);
            out.extend(strokes.into_iter().map(|s| s.iter().map(|p| [pen + p[0] * k, p[1] * k]).collect::<Vec<P>>()));
            pen += adv * k;
        }
        (out, pen)
    }
}

fn load_shx(path: &str) -> Option<Arc<ShxFont>> {
    static C: OnceLock<Mutex<HashMap<String, Arc<ShxFont>>>> = OnceLock::new();
    let c = C.get_or_init(Default::default);
    let mut m = c.lock().ok()?;
    if let Some(f) = m.get(path) {
        return Some(f.clone());
    }
    let f = Arc::new(ShxFont::parse(&std::fs::read(path).ok()?).ok()?);
    m.insert(path.into(), f.clone());
    Some(f)
}

pub fn list_shx(folder: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut v: Vec<String> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("shx")))
        .map(|p| p.to_string_lossy().to_string()).collect();
    v.sort();
    v
}

// ====================================================================== Hershey

struct Glyph {
    left: f64,
    right: f64,
    strokes: Vec<Vec<P>>,
}

struct Hershey {
    glyphs: HashMap<char, Glyph>,
    cap: f64,
    bottom: f64,
}

const HERSHEY_TAR: &[u8] = include_bytes!("../assets/hershey.tar");

fn tar_members() -> Vec<(String, &'static [u8])> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 512 <= HERSHEY_TAR.len() {
        let h = &HERSHEY_TAR[p..p + 512];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let name = String::from_utf8_lossy(&h[..100]).trim_end_matches('\0').to_string();
        let size = usize::from_str_radix(String::from_utf8_lossy(&h[124..136]).trim_matches(|c: char| c == '\0' || c == ' '), 8).unwrap_or(0);
        p += 512;
        out.push((name, &HERSHEY_TAR[p..(p + size).min(HERSHEY_TAR.len())]));
        p += size.div_ceil(512) * 512;
    }
    out
}

fn multimode_median(v: &[f64]) -> f64 {
    let mut counts: Vec<(f64, usize)> = Vec::new();
    for x in v {
        match counts.iter_mut().find(|(k, _)| k == x) {
            Some(c) => c.1 += 1,
            None => counts.push((*x, 1)),
        }
    }
    let best = counts.iter().map(|c| c.1).max().unwrap_or(0);
    let mut modes: Vec<f64> = counts.iter().filter(|c| c.1 == best).map(|c| c.0).collect();
    modes.sort_by(|a, b| a.total_cmp(b));
    let n = modes.len();
    if n == 0 { 0.0 } else if n % 2 == 1 { modes[n / 2] } else { (modes[n / 2 - 1] + modes[n / 2]) / 2.0 }
}

fn parse_hershey(data: &[u8]) -> Hershey {
    let text = String::from_utf8_lossy(data);
    let val = |c: char| c as i64 as f64 - 'R' as i64 as f64;
    let (mut glyphs, mut caps, mut bottoms) = (HashMap::new(), Vec::new(), Vec::new());
    let (mut def_cap, mut def_bottom): (Option<f64>, Option<f64>) = (None, None);
    let (mut g_cap, mut g_bottom): (Option<f64>, Option<f64>) = (None, None);
    let mut code = 32u32;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(js) = line.strip_prefix('#') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(js) {
                let g = |k: &str| v.get(k).and_then(|x| x.as_f64());
                def_cap = g("define_cap_line").or(def_cap);
                def_bottom = g("define_bottom_line").or(def_bottom);
                g_cap = g("glyph_cap_line").or(g_cap);
                g_bottom = g("glyph_bottom_line").or(g_bottom);
            }
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut strokes = Vec::new();
        let (mut left, mut right) = (0.0, 0.0);
        if chars.len() > 9 {
            let body: String = chars[10..].iter().collect();
            for s in body.split(" R") {
                let cs: Vec<char> = s.chars().collect();
                if !cs.is_empty() {
                    strokes.push(cs.chunks(2).filter(|c| c.len() == 2).map(|c| [val(c[0]), val(c[1])]).collect::<Vec<P>>());
                }
            }
            left = val(chars[8]);
            right = val(chars[9]);
        }
        caps.push(g_cap.or(def_cap).unwrap_or(-12.0));
        bottoms.push(g_bottom.or(def_bottom).unwrap_or(16.0));
        g_cap = None;
        g_bottom = None;
        if let Some(ch) = char::from_u32(code) {
            glyphs.insert(ch, Glyph { left, right, strokes });
        }
        code += 1;
    }
    Hershey { glyphs, cap: def_cap.unwrap_or_else(|| multimode_median(&caps)), bottom: def_bottom.unwrap_or_else(|| multimode_median(&bottoms)) }
}

fn hershey(name: &str) -> Option<Arc<Hershey>> {
    static C: OnceLock<Mutex<HashMap<String, Arc<Hershey>>>> = OnceLock::new();
    let c = C.get_or_init(Default::default);
    let mut m = c.lock().ok()?;
    if let Some(f) = m.get(name) {
        return Some(f.clone());
    }
    let (_, data) = tar_members().into_iter().find(|(n, _)| n == name)?;
    let f = Arc::new(parse_hershey(data));
    m.insert(name.into(), f.clone());
    Some(f)
}

pub fn hershey_names() -> Vec<String> {
    let skip = ["astrology", "markers", "mathlow", "mathupp", "meteorology", "music", "symbolic", "japanese"];
    tar_members().into_iter().map(|(n, _)| n).filter(|n| !skip.contains(&n.as_str())).collect()
}

/// model.single_line_fonts: (font id, label) for every single-line font.
pub fn single_line_fonts() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = hershey_names().into_iter().map(|n| (format!("{HERSHEY_PREFIX}{n}"), n)).collect();
    let folder = SHX_FOLDER.lock().map(|f| f.clone()).unwrap_or_default();
    for p in list_shx(&folder) {
        let stem = std::path::Path::new(&p).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        out.push((format!("{SHX_PREFIX}{p}"), format!("SHX · {stem}")));
    }
    out
}

/// model.single_line_paths: open strokes in mm; height = size, baseline 25 % above the bottom.
pub fn single_line_paths(s: &Shape) -> Vec<Vec<P>> {
    let text = crate::text::resolve_text(s);
    let size = s.f("size");
    let font = s.0.get("font").and_then(|v| v.as_str()).unwrap_or("");
    let (x, y) = (s.f("x"), s.f("y"));
    if let Some(path) = font.strip_prefix(SHX_PREFIX) {
        let Some(f) = load_shx(path) else { return Vec::new() };
        let (strokes, _) = f.text_strokes(&text, size * 0.75);
        let base = y + size * 0.75;
        return strokes.into_iter().filter(|st| st.len() >= 2).map(|st| st.iter().map(|p| [x + p[0], base - p[1]]).collect()).collect();
    }
    let Some(h) = hershey(font.strip_prefix(HERSHEY_PREFIX).unwrap_or("futural")) else { return Vec::new() };
    // normalize_rendering(size): upright, `size` from cap line to bottom line
    let k = size / (h.bottom - h.cap);
    let (sx, sy, yofs) = (k, -k, h.bottom * k);
    let mut xofs = 0.0;
    let mut out = Vec::new();
    for ch in text.chars() {
        let Some(g) = h.glyphs.get(&ch) else { continue };
        for st in &g.strokes {
            if st.len() >= 2 {
                out.push(st.iter().map(|p| {
                    let (px, py) = (xofs + (p[0] - g.left) * sx, yofs + p[1] * sy);
                    [x + px, y + size - py]
                }).collect());
            }
        }
        xofs += sx * (g.right - g.left);
    }
    out
}
