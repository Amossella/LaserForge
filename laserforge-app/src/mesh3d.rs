//! 3D model import (STL, OBJ, PLY, AMF) rendered as a top-view depth map for 3D sliced engraving: highest
//! surface = white (no burn), deepest = black, empty background = white. Ported from laserforge/mesh3d.py.

use std::path::Path;

pub type Tri = [[f64; 3]; 3];

fn stl(data: &[u8]) -> Result<Vec<Tri>, String> {
    if data.len() >= 5 && data[..5].eq_ignore_ascii_case(b"solid") && data[..data.len().min(1024)].windows(5).any(|w| w == b"facet") {
        let text = String::from_utf8_lossy(data);
        let mut v = Vec::new();
        for line in text.lines() {
            let t = line.trim();
            if let Some(r) = t.strip_prefix("vertex") {
                let n: Vec<f64> = r.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if n.len() == 3 {
                    v.push([n[0], n[1], n[2]]);
                }
            }
        }
        return Ok(v.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect());
    }
    if data.len() < 84 {
        return Err("STL troncato".into());
    }
    let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
    let f = |o: usize| f32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as f64;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o = 84 + i * 50 + 12;
        if o + 36 > data.len() {
            break;
        }
        out.push([[f(o), f(o + 4), f(o + 8)], [f(o + 12), f(o + 16), f(o + 20)], [f(o + 24), f(o + 28), f(o + 32)]]);
    }
    Ok(out)
}

fn obj(text: &str) -> Vec<Tri> {
    let (mut verts, mut tris): (Vec<[f64; 3]>, Vec<Tri>) = (Vec::new(), Vec::new());
    for line in text.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.is_empty() {
            continue;
        }
        if p[0] == "v" && p.len() >= 4 {
            verts.push([p[1].parse().unwrap_or(0.0), p[2].parse().unwrap_or(0.0), p[3].parse().unwrap_or(0.0)]);
        } else if p[0] == "f" && p.len() >= 4 {
            let idx: Vec<usize> = p[1..].iter().filter_map(|t| t.split('/').next()?.parse::<i64>().ok())
                .map(|i| if i > 0 { (i - 1) as usize } else { (verts.len() as i64 + i) as usize }).collect();
            for k in 1..idx.len().saturating_sub(1) {
                if let (Some(a), Some(b), Some(c)) = (verts.get(idx[0]), verts.get(idx[k]), verts.get(idx[k + 1])) {
                    tris.push([*a, *b, *c]);
                }
            }
        }
    }
    tris
}

fn ply(data: &[u8]) -> Result<Vec<Tri>, String> {
    let he = data.windows(10).position(|w| w == b"end_header").ok_or("PLY senza intestazione")? + 10;
    let he = he + if data.get(he..he + 2) == Some(b"\r\n") { 2 } else { 1 };
    let header = String::from_utf8_lossy(&data[..he]).to_string();
    let fmt = header.lines().find(|l| l.starts_with("format")).and_then(|l| l.split_whitespace().nth(1)).unwrap_or("ascii").to_string();
    struct El {
        name: String,
        count: usize,
        props: Vec<Vec<String>>,
    }
    let mut els: Vec<El> = Vec::new();
    for l in header.lines() {
        let p: Vec<String> = l.split_whitespace().map(|s| s.to_string()).collect();
        if p.first().map(|s| s.as_str()) == Some("element") && p.len() >= 3 {
            els.push(El { name: p[1].clone(), count: p[2].parse().unwrap_or(0), props: Vec::new() });
        } else if p.first().map(|s| s.as_str()) == Some("property") {
            if let Some(e) = els.last_mut() {
                e.props.push(p[1..].to_vec());
            }
        }
    }
    let body = &data[he..];
    let (mut verts, mut faces): (Vec<[f64; 3]>, Vec<[usize; 3]>) = (Vec::new(), Vec::new());
    let size = |t: &str| match t { "char" | "uchar" | "int8" | "uint8" => 1, "short" | "ushort" | "int16" | "uint16" => 2, "double" | "float64" => 8, _ => 4 };
    if fmt == "ascii" {
        let text = String::from_utf8_lossy(body);
        let lines: Vec<&str> = text.split('\n').collect();
        let mut pos = 0;
        for e in &els {
            let rows = &lines[pos.min(lines.len())..(pos + e.count).min(lines.len())];
            pos += e.count;
            if e.name == "vertex" {
                let names: Vec<&str> = e.props.iter().map(|p| p.last().map(|s| s.as_str()).unwrap_or("")).collect();
                let (ix, iy, iz) = (names.iter().position(|n| *n == "x").unwrap_or(0), names.iter().position(|n| *n == "y").unwrap_or(1), names.iter().position(|n| *n == "z").unwrap_or(2));
                for r in rows.iter().filter(|r| !r.trim().is_empty()) {
                    let v: Vec<f64> = r.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    verts.push([v.get(ix).copied().unwrap_or(0.0), v.get(iy).copied().unwrap_or(0.0), v.get(iz).copied().unwrap_or(0.0)]);
                }
            } else if e.name == "face" {
                for r in rows {
                    let v: Vec<usize> = r.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    if let Some(&n) = v.first() {
                        for k in 1..n.saturating_sub(1) {
                            if v.len() > k + 1 {
                                faces.push([v[1], v[1 + k], v[2 + k]]);
                            }
                        }
                    }
                }
            }
        }
    } else {
        let le = fmt.contains("little");
        let rd = |o: usize, t: &str| -> f64 {
            let b = &body[o.min(body.len())..(o + size(t)).min(body.len())];
            if b.len() < size(t) {
                return 0.0;
            }
            macro_rules! g {
                ($ty:ty) => {{
                    let a: [u8; std::mem::size_of::<$ty>()] = b.try_into().unwrap();
                    (if le { <$ty>::from_le_bytes(a) } else { <$ty>::from_be_bytes(a) }) as f64
                }};
            }
            match t {
                "char" | "int8" => b[0] as i8 as f64,
                "uchar" | "uint8" => b[0] as f64,
                "short" | "int16" => g!(i16),
                "ushort" | "uint16" => g!(u16),
                "int" | "int32" => g!(i32),
                "uint" | "uint32" => g!(u32),
                "double" | "float64" => g!(f64),
                _ => g!(f32),
            }
        };
        let mut off = 0usize;
        for e in &els {
            if e.name == "vertex" {
                let names: Vec<&str> = e.props.iter().map(|p| p.last().map(|s| s.as_str()).unwrap_or("")).collect();
                for _ in 0..e.count {
                    let mut v = [0.0; 3];
                    for (pi, p) in e.props.iter().enumerate() {
                        let val = rd(off, &p[0]);
                        match names[pi] { "x" => v[0] = val, "y" => v[1] = val, "z" => v[2] = val, _ => {} }
                        off += size(&p[0]);
                    }
                    verts.push(v);
                }
            } else if e.name == "face" && e.props.first().is_some_and(|p| p[0] == "list") {
                let (lt, it) = (e.props[0][1].clone(), e.props[0][2].clone());
                for _ in 0..e.count {
                    let n = rd(off, &lt) as usize;
                    off += size(&lt);
                    let idx: Vec<usize> = (0..n).map(|k| rd(off + k * size(&it), &it) as usize).collect();
                    off += n * size(&it);
                    for k in 1..n.saturating_sub(1) {
                        faces.push([idx[0], idx[k], idx[k + 1]]);
                    }
                }
            } else {
                let row: usize = e.props.iter().filter(|p| p[0] != "list").map(|p| size(&p[0])).sum();
                off += row * e.count;
            }
        }
    }
    Ok(faces.iter().filter_map(|f| Some([*verts.get(f[0])?, *verts.get(f[1])?, *verts.get(f[2])?])).collect())
}

fn amf(text: &str) -> Result<Vec<Tri>, String> {
    // <vertex><coordinates><x>..</x><y>..</y><z>..</z></coordinates></vertex> and <triangle><v1>..</v1>..</triangle>
    let grab = |s: &str, tag: &str| -> Vec<String> {
        let (o, c) = (format!("<{tag}>"), format!("</{tag}>"));
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(a) = rest.find(&o) {
            let r = &rest[a + o.len()..];
            let Some(b) = r.find(&c) else { break };
            out.push(r[..b].to_string());
            rest = &r[b + c.len()..];
        }
        out
    };
    let mut tris = Vec::new();
    for mesh in grab(text, "mesh") {
        let verts: Vec<[f64; 3]> = grab(&mesh, "vertex").iter().map(|v| {
            let f = |t: &str| grab(v, t).first().and_then(|x| x.trim().parse::<f64>().ok()).unwrap_or(0.0);
            [f("x"), f("y"), f("z")]
        }).collect();
        for t in grab(&mesh, "triangle") {
            let i = |k: &str| grab(&t, k).first().and_then(|x| x.trim().parse::<usize>().ok()).unwrap_or(0);
            if let (Some(a), Some(b), Some(c)) = (verts.get(i("v1")), verts.get(i("v2")), verts.get(i("v3"))) {
                tris.push([*a, *b, *c]);
            }
        }
    }
    Ok(tris)
}

/// 3MF model XML: <vertex x= y= z=/> and <triangle v1= v2= v3=/> per mesh.
fn mesh_3mf(text: &str) -> Result<Vec<Tri>, String> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(text);
    let (mut verts, mut tris): (Vec<[f64; 3]>, Vec<Tri>) = (Vec::new(), Vec::new());
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                let attr = |k: &str| e.attributes().flatten().find(|a| a.key.local_name().as_ref() == k.as_bytes()).and_then(|a| String::from_utf8_lossy(&a.value).parse::<f64>().ok());
                match name.as_str() {
                    "mesh" => verts.clear(),
                    "vertex" => verts.push([attr("x").unwrap_or(0.0), attr("y").unwrap_or(0.0), attr("z").unwrap_or(0.0)]),
                    "triangle" => {
                        let i = |k: &str| attr(k).unwrap_or(0.0) as usize;
                        if let (Some(a), Some(b), Some(c)) = (verts.get(i("v1")), verts.get(i("v2")), verts.get(i("v3"))) {
                            tris.push([*a, *b, *c]);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(e.to_string()),
            _ => {}
        }
    }
    Ok(tris)
}

pub fn load_mesh(path: &Path) -> Result<Vec<Tri>, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "stl" => stl(&data),
        "obj" => Ok(obj(&String::from_utf8_lossy(&data))),
        "ply" => ply(&data),
        "amf" if !data.starts_with(b"PK") => amf(&String::from_utf8_lossy(&data)),
        "amf" | "3mf" => {
            use std::io::Read;
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(&data)).map_err(|e| e.to_string())?;
            let name = (0..z.len()).filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string()))
                .find(|n| if ext == "3mf" { n.to_lowercase().ends_with(".model") } else { true }).ok_or("archivio vuoto")?;
            let mut text = String::new();
            z.by_name(&name).map_err(|e| e.to_string())?.read_to_string(&mut text).map_err(|e| e.to_string())?;
            if ext == "3mf" { mesh_3mf(&text) } else { amf(&text) }
        }
        _ => Err("formato 3D non supportato".into()),
    }
}

/// mesh3d.depth_map: the highest surface seen from above as grey pixels; returns (grey, cols, rows, w_mm, h_mm).
pub fn depth_map_tris(tris: &[Tri], px_mm: f64, up_y: bool, max_px: f64) -> Result<(Vec<u8>, usize, usize, f64, f64), String> {
    if tris.is_empty() {
        return Err("modello vuoto".into());
    }
    let tris: Vec<Tri> = if up_y { tris.iter().map(|t| t.map(|v| [v[0], -v[2], v[1]])).collect() } else { tris.to_vec() };
    let mut mn = [f64::MAX; 3];
    let mut mx = [f64::MIN; 3];
    for t in &tris {
        for v in t {
            for k in 0..3 {
                mn[k] = mn[k].min(v[k]);
                mx[k] = mx[k].max(v[k]);
            }
        }
    }
    let size = [mx[0] - mn[0], mx[1] - mn[1]];
    let res = px_mm.max(size[0].max(size[1]) / max_px);
    let w = ((size[0] / res).ceil() as usize + 1).max(1);
    let h = ((size[1] / res).ceil() as usize + 1).max(1);
    let mut z = vec![f64::NEG_INFINITY; w * h];
    for t in &tris {
        let gx = [(t[0][0] - mn[0]) / res, (t[1][0] - mn[0]) / res, (t[2][0] - mn[0]) / res];
        let gy = [(mx[1] - t[0][1]) / res, (mx[1] - t[1][1]) / res, (mx[1] - t[2][1]) / res];
        let gz = [t[0][2], t[1][2], t[2][2]];
        let den = (gy[1] - gy[2]) * (gx[0] - gx[2]) + (gx[2] - gx[1]) * (gy[0] - gy[2]);
        if den.abs() < 1e-12 {
            continue;
        }
        let xa = gx.iter().copied().fold(f64::MAX, f64::min).floor().max(0.0) as usize;
        let xb = (gx.iter().copied().fold(f64::MIN, f64::max).ceil() as i64).min(w as i64 - 1);
        let ya = gy.iter().copied().fold(f64::MAX, f64::min).floor().max(0.0) as usize;
        let yb = (gy.iter().copied().fold(f64::MIN, f64::max).ceil() as i64).min(h as i64 - 1);
        if xb < xa as i64 || yb < ya as i64 {
            continue;
        }
        for yy in ya..=yb as usize {
            for xx in xa..=xb as usize {
                let (px, py) = (xx as f64 + 0.5, yy as f64 + 0.5);
                let l0 = ((gy[1] - gy[2]) * (px - gx[2]) + (gx[2] - gx[1]) * (py - gy[2])) / den;
                let l1 = ((gy[2] - gy[0]) * (px - gx[2]) + (gx[0] - gx[2]) * (py - gy[2])) / den;
                let l2 = 1.0 - l0 - l1;
                if l0 >= -1e-9 && l1 >= -1e-9 && l2 >= -1e-9 {
                    let zz = l0 * gz[0] + l1 * gz[1] + l2 * gz[2];
                    let k = yy * w + xx;
                    if zz > z[k] {
                        z[k] = zz;
                    }
                }
            }
        }
    }
    let cov: Vec<f64> = z.iter().copied().filter(|v| v.is_finite()).collect();
    if cov.is_empty() {
        return Err("il modello non ha superfici visibili dall'alto".into());
    }
    let (zmin, zmax) = (cov.iter().copied().fold(f64::MAX, f64::min), cov.iter().copied().fold(f64::MIN, f64::max));
    let span = if zmax > zmin { zmax - zmin } else { 1.0 };
    let grey: Vec<u8> = z.iter().map(|v| if v.is_finite() { ((v - zmin) / span * 255.0) as u8 } else { 255 }).collect();
    Ok((grey, w, h, w as f64 * res, h as f64 * res))
}

/// Depth map of a model file at `px_mm` resolution, as a PNG data URL plus its size in mm.
pub fn depth_map(path: &Path, px_mm: f64, invert: bool) -> Result<(String, f64, f64), String> {
    let tris = load_mesh(path)?;
    let up_y = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("obj"));
    let (mut grey, w, h, wmm, hmm) = depth_map_tris(&tris, px_mm, up_y, 1600.0)?;
    if invert {
        for g in grey.iter_mut() {
            *g = 255 - *g;
        }
    }
    let img = image::GrayImage::from_raw(w as u32, h as u32, grey).ok_or("immagine non valida")?;
    Ok((crate::importers::encode_image(&image::DynamicImage::ImageLuma8(img)), wmm, hmm))
}
