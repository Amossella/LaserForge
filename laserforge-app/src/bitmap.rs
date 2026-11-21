//! Bitmap edits on image shapes: bake a crop, rotate by quarter turns or any angle, convert vectors to a bitmap.
//! Ported from laserforge/model.py (flatten_crop, rotate_image_90, rotate_image_any) and
//! mainwindow.convert_to_bitmap.

use crate::edit::{center, rotate_point};
use crate::importers::encode_image;
use crate::shapes::{flatten, is_closed_path, multi_bounds, Shape, P};
use image::{DynamicImage, Rgba, RgbaImage};
use serde_json::json;

/// The shape's picture with its flips applied (alpha kept).
pub fn picture(s: &Shape) -> Option<RgbaImage> {
    use base64::Engine;
    let src = s.0.get("src")?.as_str()?;
    let b64 = src.split_once(',').map(|x| x.1).unwrap_or(src);
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    let mut img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    if s.flag("flipX") {
        img = image::imageops::flip_horizontal(&img);
    }
    if s.flag("flipY") {
        img = image::imageops::flip_vertical(&img);
    }
    Some(img)
}

/// model.flatten_crop: white outside the crop, trimmed to the crop's box.
pub fn flatten_crop(s: &mut Shape) {
    let rings: Option<Vec<Vec<P>>> = s.0.get("crop").and_then(|c| serde_json::from_value(c.clone()).ok());
    let (Some(rings), Some(img)) = (rings, picture(s)) else {
        s.0.remove("crop");
        return;
    };
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mask = crate::edit::mask(&rings, w, h);
    let mut out = img.clone();
    for (i, p) in out.pixels_mut().enumerate() {
        if mask[i] == 0 {
            *p = Rgba([255, 255, 255, 255]);
        }
    }
    let us: Vec<f64> = rings.iter().flatten().map(|q| q[0].clamp(0.0, 1.0)).collect();
    let vs: Vec<f64> = rings.iter().flatten().map(|q| q[1].clamp(0.0, 1.0)).collect();
    let (umin, umax) = (us.iter().copied().fold(1.0, f64::min), us.iter().copied().fold(0.0, f64::max));
    let (vmin, vmax) = (vs.iter().copied().fold(1.0, f64::min), vs.iter().copied().fold(0.0, f64::max));
    let x0 = (umin * w as f64) as u32;
    let y0 = (vmin * h as f64) as u32;
    let x1 = (x0 + 1).max((umax * w as f64).ceil() as u32).min(w as u32);
    let y1 = (y0 + 1).max((vmax * h as f64).ceil() as u32).min(h as u32);
    let cropped = image::imageops::crop_imm(&out, x0, y0, x1 - x0, y1 - y0).to_image();
    let (sx, sy, sw, sh) = (s.f("x"), s.f("y"), s.f("w"), s.f("h"));
    s.0.insert("src".into(), json!(encode_image(&DynamicImage::ImageRgba8(cropped))));
    s.set("x", sx + x0 as f64 / w as f64 * sw);
    s.set("y", sy + y0 as f64 / h as f64 * sh);
    s.set("w", (x1 - x0) as f64 / w as f64 * sw);
    s.set("h", (y1 - y0) as f64 / h as f64 * sh);
    s.0.insert("flipX".into(), json!(false));
    s.0.insert("flipY".into(), json!(false));
    s.0.remove("crop");
}

/// Bicubic sample (white outside), like Pillow's rotate with BICUBIC.
fn sample(img: &RgbaImage, x: f64, y: f64) -> Rgba<u8> {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (x, y) = (x - 0.5, y - 0.5);
    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - ix as f64, y - iy as f64);
    let k = |t: f64| {
        let a = -0.5;
        let t = t.abs();
        if t <= 1.0 { (a + 2.0) * t * t * t - (a + 3.0) * t * t + 1.0 } else if t < 2.0 { a * t * t * t - 5.0 * a * t * t + 8.0 * a * t - 4.0 * a } else { 0.0 }
    };
    if ix < -1 || iy < -1 || ix > w || iy > h {
        return Rgba([255, 255, 255, 255]);
    }
    let mut acc = [0.0f64; 4];
    for j in -1..=2 {
        for i in -1..=2 {
            let (px, py) = (ix + i, iy + j);
            let c = if px < 0 || py < 0 || px >= w || py >= h { Rgba([255, 255, 255, 255]) } else { *img.get_pixel(px as u32, py as u32) };
            let wgt = k(i as f64 - fx) * k(j as f64 - fy);
            for ch in 0..4 {
                acc[ch] += c.0[ch] as f64 * wgt;
            }
        }
    }
    Rgba([acc[0].round().clamp(0.0, 255.0) as u8, acc[1].round().clamp(0.0, 255.0) as u8, acc[2].round().clamp(0.0, 255.0) as u8,
          acc[3].round().clamp(0.0, 255.0) as u8])
}

/// model.rotate_image_90 / rotate_image_any: bake the rotation into the bitmap, keeping the pixel size in mm.
pub fn rotate_image(s: &mut Shape, cx: f64, cy: f64, deg: f64) {
    flatten_crop(s);
    let Some(img) = picture(s) else { return };
    let b = s.bounds();
    let n = rotate_point(center(b), cx, cy, deg);
    let (out, w, h) = if deg.rem_euclid(90.0) == 0.0 {
        let turns = ((deg / 90.0).round() as i64).rem_euclid(4);
        let out = match turns {
            1 => image::imageops::rotate90(&img),
            2 => image::imageops::rotate180(&img),
            3 => image::imageops::rotate270(&img),
            _ => img,
        };
        let (w, h) = if turns % 2 == 1 { (b[3], b[2]) } else { (b[2], b[3]) };
        (out, w, h)
    } else {
        let (kx, ky) = (img.width() as f64 / s.f("w").max(1e-6), img.height() as f64 / s.f("h").max(1e-6));
        let k = kx.max(ky);
        let img = if (kx - ky).abs() > 1e-3 * k {
            image::imageops::resize(&img, ((s.f("w") * k).round() as u32).max(1), ((s.f("h") * k).round() as u32).max(1), image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        let a = deg * (std::f64::consts::PI / 180.0);
        let (c, sn) = (a.cos(), a.sin());
        let (iw, ih) = (img.width() as f64, img.height() as f64);
        let nw = (iw * c.abs() + ih * sn.abs()).ceil().max(1.0);
        let nh = (iw * sn.abs() + ih * c.abs()).ceil().max(1.0);
        let mut out = RgbaImage::new(nw as u32, nh as u32);
        for (x, y, p) in out.enumerate_pixels_mut() {
            // inverse mapping from output pixel centre to source
            let (dx, dy) = (x as f64 + 0.5 - nw / 2.0, y as f64 + 0.5 - nh / 2.0);
            let (sx, sy) = (dx * c + dy * sn + iw / 2.0, -dx * sn + dy * c + ih / 2.0);
            *p = sample(&img, sx, sy);
        }
        let (w, h) = (nw / k, nh / k);
        (out, w, h)
    };
    s.0.insert("src".into(), json!(encode_image(&DynamicImage::ImageRgba8(out))));
    s.set("x", n[0] - w / 2.0);
    s.set("y", n[1] - h / 2.0);
    s.set("w", w);
    s.set("h", h);
    s.0.insert("flipX".into(), json!(false));
    s.0.insert("flipY".into(), json!(false));
}

/// mainwindow.convert_to_bitmap: render vector shapes to a black-on-white image (fill closed shapes even-odd,
/// stroke the rest). Returns the image shape's (src, x, y, w, h) or an error message.
pub fn vectors_to_bitmap(sel: &[Shape], px_mm: f64, fill: bool, stroke_mm: f64) -> Result<(String, f64, f64, f64, f64), String> {
    let b = multi_bounds(sel).ok_or_else(|| crate::tr("selectFirst"))?;
    let pad = stroke_mm.max(px_mm);
    let (x0, y0) = (b[0] - pad, b[1] - pad);
    let w = ((b[2] + 2.0 * pad) / px_mm).round().max(1.0) as usize;
    let h = ((b[3] + 2.0 * pad) / px_mm).round().max(1.0) as usize;
    if w * h > 40_000_000 {
        return Err(crate::l("Immagine troppo grande: aumenta la dimensione del pixel", "Image too large: increase the pixel size"));
    }
    let mut ink = vec![0u8; w * h];
    let lw = (stroke_mm / px_mm).round().max(1.0);
    for s in flatten(sel) {
        for path in s.paths() {
            let pts: Vec<P> = path.iter().map(|q| [(q[0] - x0) / px_mm, (q[1] - y0) / px_mm]).collect();
            if pts.len() < 2 {
                continue;
            }
            if fill && is_closed_path(&path) && pts.len() > 3 {
                let ring: Vec<P> = pts.iter().map(|q| [q[0] / w as f64, q[1] / h as f64]).collect();
                let m = crate::edit::mask(&[ring], w, h);
                for (a, b) in ink.iter_mut().zip(m) {
                    *a ^= b;
                }
            } else {
                for seg in pts.windows(2) {
                    let len = (seg[1][0] - seg[0][0]).hypot(seg[1][1] - seg[0][1]);
                    let steps = (len * 2.0).ceil().max(1.0) as usize;
                    for k in 0..=steps {
                        let t = k as f64 / steps as f64;
                        let (cx, cy) = (seg[0][0] + (seg[1][0] - seg[0][0]) * t, seg[0][1] + (seg[1][1] - seg[0][1]) * t);
                        let r = lw / 2.0;
                        for yy in ((cy - r).floor() as i64)..=((cy + r).ceil() as i64) {
                            for xx in ((cx - r).floor() as i64)..=((cx + r).ceil() as i64) {
                                if xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h
                                    && (xx as f64 + 0.5 - cx).hypot(yy as f64 + 0.5 - cy) <= r.max(0.5) {
                                    ink[yy as usize * w + xx as usize] = 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let img = image::GrayImage::from_raw(w as u32, h as u32, ink.iter().map(|&v| if v > 0 { 0 } else { 255 }).collect()).unwrap();
    Ok((encode_image(&DynamicImage::ImageLuma8(img)), x0, y0, w as f64 * px_mm, h as f64 * px_mm))
}

/// model.trace_binary: marching squares on a boolean grid (rows × cols); closed loops in pixel-corner coordinates.
pub fn trace_binary(on: &[bool], cols: usize, rows: usize) -> Vec<Vec<P>> {
    let at = |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < cols && (y as usize) < rows && on[y as usize * cols + x as usize];
    let mut adj: std::collections::BTreeMap<(i64, i64), Vec<(i64, i64)>> = Default::default();
    let mut order: Vec<(i64, i64)> = Vec::new();
    let link = |a: (i64, i64), b: (i64, i64), adj: &mut std::collections::BTreeMap<(i64, i64), Vec<(i64, i64)>>, order: &mut Vec<(i64, i64)>| {
        for (p, q) in [(a, b), (b, a)] {
            adj.entry(p).or_insert_with(|| { order.push(p); Vec::new() }).push(q);
        }
    };
    for yy in 0..=rows as i64 {
        for xx in 0..=cols as i64 {
            let (x, y) = (xx - 1, yy - 1);
            let v = (at(x, y) as u8) * 8 + (at(x + 1, y) as u8) * 4 + (at(x + 1, y + 1) as u8) * 2 + at(x, y + 1) as u8;
            if v == 0 || v == 15 {
                continue;
            }
            let (t, r, b, l) = ((2 * x + 1, 2 * y), (2 * x + 2, 2 * y + 1), (2 * x + 1, 2 * y + 2), (2 * x, 2 * y + 1));
            match v {
                1 | 14 => link(l, b, &mut adj, &mut order),
                2 | 13 => link(b, r, &mut adj, &mut order),
                3 | 12 => link(l, r, &mut adj, &mut order),
                4 | 11 => link(t, r, &mut adj, &mut order),
                6 | 9 => link(t, b, &mut adj, &mut order),
                7 | 8 => link(t, l, &mut adj, &mut order),
                5 => { link(t, l, &mut adj, &mut order); link(b, r, &mut adj, &mut order); }
                10 => { link(t, r, &mut adj, &mut order); link(l, b, &mut adj, &mut order); }
                _ => {}
            }
        }
    }
    let mut loops = Vec::new();
    for start in order {
        while let Some(nxt0) = adj.get_mut(&start).and_then(|l| l.pop()) {
            let mut path = vec![start];
            let (mut cur, mut nxt) = (start, nxt0);
            if let Some(l) = adj.get_mut(&nxt) {
                if let Some(k) = l.iter().position(|p| *p == cur) {
                    l.remove(k);
                }
            }
            while nxt != start {
                path.push(nxt);
                let Some(n2) = adj.get_mut(&nxt).and_then(|l| l.pop()) else { break };
                if let Some(l) = adj.get_mut(&n2) {
                    if let Some(k) = l.iter().position(|p| *p == nxt) {
                        l.remove(k);
                    }
                }
                cur = nxt;
                nxt = n2;
            }
            let _ = cur;
            if nxt == start {
                path.push(start);
            }
            if path.len() > 3 {
                loops.push(path.iter().map(|p| [p.0 as f64 / 2.0 + 0.5, p.1 as f64 / 2.0 + 0.5]).collect());
            }
        }
    }
    loops
}
