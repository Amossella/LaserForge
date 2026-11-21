//! Text shapes: glyph outlines from the system fonts (TrueType / OpenType, read with ttf-parser), laid out
//! straight, with letter spacing or along an arc. Ported from laserforge/model.py (text_paths, text_width,
//! text_on_path, resolve_text). The Python app uses Qt's font engine, so outlines match in shape and size but
//! not point for point (different curve flattening, no kerning here).

use crate::shapes::{Shape, P};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Qt pixel size the Python app lays text out at; outlines are scaled by size / FONT_PX.
const FONT_PX: f64 = 100.0;

#[derive(Debug, Clone)]
pub struct FontFace {
    pub path: PathBuf,
    pub index: u32,
    pub family: String,
    pub bold: bool,
    pub italic: bool,
}

fn font_dirs() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/System/Library/Fonts"), PathBuf::from("/System/Library/Fonts/Supplemental"),
                     PathBuf::from("/Library/Fonts"), PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts"),
                     PathBuf::from("C:\\Windows\\Fonts")];
    if let Some(h) = std::env::var_os("HOME") {
        v.push(PathBuf::from(&h).join("Library/Fonts"));
        v.push(PathBuf::from(&h).join(".fonts"));
        v.push(PathBuf::from(&h).join(".local/share/fonts"));
    }
    v
}

fn scan(dir: &PathBuf, out: &mut Vec<FontFace>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 3 {
            scan(&p, out, depth + 1);
            continue;
        }
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
            continue;
        }
        let Ok(data) = std::fs::read(&p) else { continue };
        let n = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
        for i in 0..n {
            let Ok(face) = ttf_parser::Face::parse(&data, i) else { continue };
            let name = |id: u16| face.names().into_iter().filter(|n| n.name_id == id && n.is_unicode()).find_map(|n| n.to_string());
            let Some(family) = name(ttf_parser::name_id::TYPOGRAPHIC_FAMILY).or_else(|| name(ttf_parser::name_id::FAMILY)) else { continue };
            out.push(FontFace { path: p.clone(), index: i, family, bold: face.is_bold(), italic: face.is_italic() });
        }
    }
}

/// Every font face installed on the system (scanned once).
pub fn faces() -> &'static [FontFace] {
    static F: OnceLock<Vec<FontFace>> = OnceLock::new();
    F.get_or_init(|| {
        let mut v = Vec::new();
        for d in font_dirs() {
            scan(&d, &mut v, 0);
        }
        v.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()));
        v
    })
}

/// Font family names for the font picker.
pub fn families() -> Vec<String> {
    let mut v: Vec<String> = faces().iter().map(|f| f.family.clone()).collect();
    v.dedup();
    v
}

fn data(path: &PathBuf) -> Option<Arc<Vec<u8>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>> = OnceLock::new();
    let c = CACHE.get_or_init(Default::default);
    let mut m = c.lock().ok()?;
    if let Some(d) = m.get(path) {
        return Some(d.clone());
    }
    let d = Arc::new(std::fs::read(path).ok()?);
    m.insert(path.clone(), d.clone());
    Some(d)
}

/// The best face for a family (Helvetica when empty) and style; falls back to any face.
fn pick(family: &str, bold: bool, italic: bool) -> Option<&'static FontFace> {
    let fam = if family.is_empty() { "Helvetica" } else { family };
    let all = faces();
    let same: Vec<&FontFace> = all.iter().filter(|f| f.family.eq_ignore_ascii_case(fam)).collect();
    let pool: Vec<&FontFace> = if same.is_empty() {
        let alt: Vec<&FontFace> = all.iter().filter(|f| ["Helvetica", "Arial", "DejaVu Sans", "Liberation Sans"].iter().any(|a| f.family == *a)).collect();
        if alt.is_empty() { all.iter().collect() } else { alt }
    } else {
        same
    };
    pool.iter().copied().min_by_key(|f| (f.bold != bold) as u8 * 2 + (f.italic != italic) as u8)
}

struct Outline {
    rings: Vec<Vec<P>>,
    cur: Vec<P>,
    scale: f64,
}

impl Outline {
    fn p(&self, x: f32, y: f32) -> P {
        [x as f64 * self.scale, -(y as f64) * self.scale]
    }
    fn close_ring(&mut self) {
        if self.cur.len() > 2 {
            if self.cur[0] != *self.cur.last().unwrap() {
                let f = self.cur[0];
                self.cur.push(f);
            }
            self.rings.push(std::mem::take(&mut self.cur));
        }
        self.cur.clear();
    }
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close_ring();
        let p = self.p(x, y);
        self.cur.push(p);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.p(x, y);
        self.cur.push(p);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (p0, c, e) = (*self.cur.last().unwrap_or(&[0.0, 0.0]), self.p(x1, y1), self.p(x, y));
        for k in 1..=8 {
            let t = k as f64 / 8.0;
            let u = 1.0 - t;
            self.cur.push([u * u * p0[0] + 2.0 * u * t * c[0] + t * t * e[0], u * u * p0[1] + 2.0 * u * t * c[1] + t * t * e[1]]);
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (p0, c1, c2, e) = (*self.cur.last().unwrap_or(&[0.0, 0.0]), self.p(x1, y1), self.p(x2, y2), self.p(x, y));
        for k in 1..=10 {
            let t = k as f64 / 10.0;
            let u = 1.0 - t;
            let f = |a: f64, b: f64, c: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
            self.cur.push([f(p0[0], c1[0], c2[0], e[0]), f(p0[1], c1[1], c2[1], e[1])]);
        }
    }
    fn close(&mut self) {
        self.close_ring();
    }
}

/// Glyphs of a string at Qt's 100 px size: (outline rings relative to the pen at the baseline, advance) per char.
fn glyphs(s: &Shape, text: &str) -> Vec<(char, Vec<Vec<P>>, f64)> {
    let family = s.0.get("font").and_then(|v| v.as_str()).unwrap_or("");
    let Some(ff) = pick(family, s.flag("bold"), s.flag("italic")) else { return Vec::new() };
    let Some(d) = data(&ff.path) else { return Vec::new() };
    let Ok(face) = ttf_parser::Face::parse(&d, ff.index) else { return Vec::new() };
    let scale = FONT_PX / face.units_per_em() as f64;
    let ids: Vec<Option<ttf_parser::GlyphId>> = text.chars().map(|ch| face.glyph_index(ch).or_else(|| face.glyph_index('?'))).collect();
    // pair kerning from the 'kern' table (Qt kerns by default)
    let kern = |a: ttf_parser::GlyphId, b: ttf_parser::GlyphId| -> f64 {
        face.tables().kern.map(|k| k.subtables.into_iter().filter(|st| st.horizontal && !st.variable)
            .filter_map(|st| st.glyphs_kerning(a, b)).map(|v| v as f64).sum::<f64>()).unwrap_or(0.0)
    };
    text.chars().enumerate().map(|(i, ch)| {
        let gid = ids[i];
        let mut o = Outline { rings: Vec::new(), cur: Vec::new(), scale };
        let mut adv = gid.and_then(|g| {
            face.outline_glyph(g, &mut o);
            face.glyph_hor_advance(g)
        }).unwrap_or(0) as f64;
        if let (Some(a), Some(Some(b))) = (gid, ids.get(i + 1)) {
            adv += kern(a, *b);
        }
        o.close_ring();
        (ch, o.rings, adv * scale)
    }).collect()
}

/// model.resolve_text: variable text ({serial}/{n}, {date}, {time}, {csv:Col} / {Col}).
pub fn resolve_text(s: &Shape) -> String {
    let text = s.0.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let Some(var) = s.0.get("var").filter(|v| v.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false)) else { return text };
    let rows = var.get("csv").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let row = if rows.is_empty() { None } else { rows.get(var.get("row").and_then(|v| v.as_u64()).unwrap_or(0) as usize % rows.len()) };
    let (day, month, year, hour, min) = local_now();
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(a) = rest.find('{') {
        let Some(b) = rest[a + 1..].find(['}', '{']).map(|b| b + a + 1) else { break };
        if rest.as_bytes()[b] == b'{' {
            out.push_str(&rest[..b]);
            rest = &rest[b..];
            continue;
        }
        out.push_str(&rest[..a]);
        let raw = &rest[a + 1..b];
        let key = raw.trim();
        let low = key.to_lowercase();
        let rep = if matches!(low.as_str(), "serial" | "n" | "seriale") {
            let n = var.get("serial").and_then(|v| v.as_f64()).unwrap_or(1.0) as i64;
            let digits = var.get("digits").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
            Some(format!("{n:0digits$}"))
        } else if matches!(low.as_str(), "date" | "data") {
            let fmt = var.get("date_fmt").and_then(|v| v.as_str()).unwrap_or("%d/%m/%Y");
            Some(fmt.replace("%d", &format!("{day:02}")).replace("%m", &format!("{month:02}")).replace("%Y", &year.to_string())
                 .replace("%y", &format!("{:02}", year % 100)))
        } else if matches!(low.as_str(), "time" | "ora") {
            Some(format!("{hour:02}:{min:02}"))
        } else {
            let k = if low.starts_with("csv:") { key[4..].trim() } else { key };
            row.and_then(|r| r.get(k)).map(|v| v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string()))
        };
        match rep {
            Some(r) => out.push_str(&r),
            None => out.push_str(&rest[a..=b]),
        }
        rest = &rest[b + 1..];
    }
    out.push_str(rest);
    out
}

/// Local date and time (day, month, year, hour, minute) from the system clock and the TZ offset of `date +%z`.
fn local_now() -> (u32, u32, i64, u32, u32) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    static OFF: OnceLock<i64> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::process::Command::new("date").arg("+%z").output().ok().and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let sign = if s.starts_with('-') { -1 } else { 1 };
            let v: i64 = s.trim_start_matches(['+', '-']).parse().ok()?;
            Some(sign * ((v / 100) * 3600 + (v % 100) * 60))
        }).unwrap_or(0)
    });
    let t = secs + off;
    let (days, rem) = (t.div_euclid(86400), t.rem_euclid(86400));
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + (m <= 2) as i64;
    (d, m, y, (rem / 3600) as u32, (rem % 3600 / 60) as u32)
}

/// model.text_width
pub fn text_width(s: &Shape) -> f64 {
    let text = resolve_text(s);
    let k = s.f("size") / FONT_PX;
    let w: f64 = glyphs(s, &text).iter().map(|g| g.2).sum::<f64>() * k;
    w + s.f("spacing") * (text.chars().count().saturating_sub(1)) as f64
}

/// model.text_on_path: glyphs laid along a polyline (closed polygons).
pub fn text_on_path(s: &Shape, path: &[P], start: f64, distance: f64, spacing: f64, flip: bool) -> Vec<Vec<P>> {
    let mut pts = path.to_vec();
    if flip {
        pts.reverse();
    }
    let seglen: Vec<f64> = pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).collect();
    let total: f64 = seglen.iter().sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let at = |d: f64| -> (f64, f64, f64) {
        let mut d = d.clamp(0.0, total);
        for (i, l) in seglen.iter().enumerate() {
            if d <= *l || i == seglen.len() - 1 {
                let k = if *l != 0.0 { d / l } else { 0.0 };
                let (a, b) = (pts[i], pts[i + 1]);
                return (a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, (b[1] - a[1]).atan2(b[0] - a[0]));
            }
            d -= l;
        }
        (pts[pts.len() - 1][0], pts[pts.len() - 1][1], 0.0)
    };
    let k = s.f("size") / FONT_PX;
    let text = resolve_text(s);
    let mut out = Vec::new();
    let mut pos = start;
    for (ch, rings, adv) in glyphs(s, &text) {
        let adv = adv * k;
        if !ch.is_whitespace() {
            let (x, y, ang) = at(pos + adv / 2.0);
            let (ca, sa) = (ang.cos(), ang.sin());
            for ring in rings {
                let mut r: Vec<P> = ring.iter().map(|p| {
                    let (gx, gy) = (p[0] * k - adv / 2.0, p[1] * k - distance);
                    [x + gx * ca - gy * sa, y + gx * sa + gy * ca]
                }).collect();
                if r.len() > 2 {
                    if r[0] != *r.last().unwrap() {
                        r.push(r[0]);
                    }
                    out.push(r);
                }
            }
        }
        pos += adv + spacing;
    }
    out
}

/// model._text_baseline_path
fn baseline(s: &Shape) -> Vec<P> {
    let w = text_width(s);
    let (x0, base) = (s.f("x"), s.f("y") + s.f("size") * 0.8);
    let r = s.f("bend");
    if r.abs() < 1e-6 {
        return vec![[x0, base], [x0 + w, base]];
    }
    let xc = x0 + w / 2.0;
    let n = 16.max((w / 0.5) as usize);
    (0..=n).map(|i| {
        let d = -w / 2.0 + w * i as f64 / n as f64;
        let t = d / r;
        [xc + r * t.sin(), base + r * (1.0 - t.cos())]
    }).collect()
}

pub fn is_single_line(s: &Shape) -> bool {
    let f = s.0.get("font").and_then(|v| v.as_str()).unwrap_or("");
    f.starts_with("Hershey:") || f.starts_with("SHX:")
}

/// model.text_paths: closed glyph outlines in mm.
pub fn text_paths(s: &Shape) -> Vec<Vec<P>> {
    if is_single_line(s) {
        return crate::shx::single_line_paths(s);
    }
    if s.f("spacing") != 0.0 || s.f("bend") != 0.0 {
        return text_on_path(s, &baseline(s), 0.0, 0.0, s.f("spacing"), false);
    }
    let text = resolve_text(s);
    let k = s.f("size") / FONT_PX;
    let (ox, oy) = (s.f("x"), s.f("y") + s.f("size") * 0.8);
    let mut out = Vec::new();
    let mut pen = 0.0;
    for (_, rings, adv) in glyphs(s, &text) {
        for ring in rings {
            if ring.len() > 2 {
                out.push(ring.iter().map(|p| [ox + (pen + p[0]) * k, oy + p[1] * k]).collect());
            }
        }
        pen += adv;
    }
    out
}

/// model.bounds for text
pub fn text_bounds(s: &Shape) -> [f64; 4] {
    if s.f("bend") != 0.0 || is_single_line(s) {
        let p: Vec<P> = text_paths(s).into_iter().flatten().collect();
        if !p.is_empty() {
            if is_single_line(s) {
                let b = crate::shapes::path_box(&p);
                return [s.f("x"), s.f("y"), b[0] + b[2] - s.f("x"), s.f("size")];
            }
            return crate::shapes::path_box(&p);
        }
    }
    [s.f("x"), s.f("y"), text_width(s), s.f("size")]
}
