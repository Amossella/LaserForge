//! Camera: lens correction, alignment (4 corners or burned targets), top-down warp of the work area, mark
//! detection, and frame capture. Maths ported from laserforge/extras.py; capture goes through `nokhwa`
//! (any frame source works: a still image can stand in for the camera).

use image::{Rgb, RgbImage};
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub type H3 = [[f64; 3]; 3];

/// extras.undistort: each output pixel samples the source at r·(1 + k1 r² + k2 r⁴) (r normalised to the
/// half-diagonal), bilinear. Negative k1 fixes barrel distortion.
pub fn undistort(img: &RgbImage, k1: f64, k2: f64) -> RgbImage {
    if k1 == 0.0 && k2 == 0.0 {
        return img.clone();
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (cx, cy, norm) = ((w as f64 - 1.0) / 2.0, (h as f64 - 1.0) / 2.0, (w as f64 / 2.0).hypot(h as f64 / 2.0));
    let mut out = RgbImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            // float32 like numpy: keep the same rounding as the Python code
            let (nx, ny) = (((x as f32 - cx as f32) / norm as f32) as f64, ((y as f32 - cy as f32) / norm as f32) as f64);
            let r2 = nx * nx + ny * ny;
            let f = 1.0 + k1 * r2 + k2 * r2 * r2;
            let sx = (cx + nx * f * norm).clamp(0.0, w as f64 - 1.001);
            let sy = (cy + ny * f * norm).clamp(0.0, h as f64 - 1.001);
            let (x0, y0) = (sx as usize, sy as usize);
            let (fx, fy) = (sx - x0 as f64, sy - y0 as f64);
            let px = |xx: usize, yy: usize| img.get_pixel(xx.min(w - 1) as u32, yy.min(h - 1) as u32).0;
            let (a, b, c, d) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
            let mut o = [0u8; 3];
            for k in 0..3 {
                let v = a[k] as f64 * (1.0 - fx) * (1.0 - fy) + b[k] as f64 * fx * (1.0 - fy) + c[k] as f64 * (1.0 - fx) * fy + d[k] as f64 * fx * fy;
                o[k] = v.clamp(0.0, 255.0) as u8;
            }
            out.put_pixel(x as u32, y as u32, Rgb(o));
        }
    }
    out
}

#[allow(dead_code)]
/// Solve a small dense linear system (Gaussian elimination with partial pivoting).
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-14 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in 0..n {
            if r != c {
                let f = a[r][c] / a[c][c];
                for k in c..n {
                    a[r][k] -= f * a[c][k];
                }
                b[r] -= f * b[c];
            }
        }
    }
    Some((0..n).map(|i| b[i] / a[i][i]).collect())
}

/// Eigenvector of the smallest eigenvalue of a symmetric matrix (cyclic Jacobi).
fn smallest_eigvec(mut a: Vec<Vec<f64>>) -> Vec<f64> {
    let n = a.len();
    let mut v = vec![vec![0.0; n]; n];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..100 {
        let off: f64 = (0..n).flat_map(|i| (0..n).filter(move |j| *j != i).map(move |j| (i, j))).map(|(i, j)| a[i][j] * a[i][j]).sum();
        if off < 1e-22 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let (c, s) = (1.0 / (t * t + 1.0).sqrt(), t / (t * t + 1.0).sqrt());
                for k in 0..n {
                    let (akp, akq) = (a[k][p], a[k][q]);
                    a[k][p] = c * akp - s * akq;
                    a[k][q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p][k], a[q][k]);
                    a[p][k] = c * apk - s * aqk;
                    a[q][k] = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let (vp, vq) = (row[p], row[q]);
                    row[p] = c * vp - s * vq;
                    row[q] = s * vp + c * vq;
                }
            }
        }
    }
    let k = (0..n).min_by(|&i, &j| a[i][i].total_cmp(&a[j][j])).unwrap();
    v.iter().map(|row| row[k]).collect()
}

/// extras.homography: 3×3 matrix mapping src points to dst points (DLT: the right singular vector of the
/// smallest singular value, ≥ 4 pairs), normalised so h33 = 1.
pub fn homography(src: &[[f64; 2]], dst: &[[f64; 2]]) -> Option<H3> {
    if src.len() < 4 || src.len() != dst.len() {
        return None;
    }
    let mut rows: Vec<[f64; 9]> = Vec::new();
    for (s, d) in src.iter().zip(dst) {
        let (x, y, u, v) = (s[0], s[1], d[0], d[1]);
        rows.push([-x, -y, -1.0, 0.0, 0.0, 0.0, u * x, u * y, u]);
        rows.push([0.0, 0.0, 0.0, -x, -y, -1.0, v * x, v * y, v]);
    }
    let mut ata = vec![vec![0.0; 9]; 9];
    for r in &rows {
        for i in 0..9 {
            for j in 0..9 {
                ata[i][j] += r[i] * r[j];
            }
        }
    }
    let h = smallest_eigvec(ata);
    if h[8].abs() < 1e-15 {
        return None;
    }
    let h: Vec<f64> = h.iter().map(|x| x / h[8]).collect();
    Some([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], h[8]]])
}

pub fn apply_h(m: &H3, x: f64, y: f64) -> [f64; 2] {
    let w = m[2][0] * x + m[2][1] * y + m[2][2];
    [(m[0][0] * x + m[0][1] * y + m[0][2]) / w, (m[1][0] * x + m[1][1] * y + m[1][2]) / w]
}

pub fn invert(m: &H3) -> Option<H3> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-15 {
        return None;
    }
    let c = |a: usize, b: usize, c2: usize, d: usize| m[a][b] * m[c2][d] - m[a][d] * m[c2][b];
    Some([[c(1, 1, 2, 2) / det, -c(0, 1, 2, 2) / det, c(0, 1, 1, 2) / det],
          [-c(1, 0, 2, 2) / det, c(0, 0, 2, 2) / det, -c(0, 0, 1, 2) / det],
          [c(1, 0, 2, 1) / det, -c(0, 0, 2, 1) / det, c(0, 0, 1, 1) / det]])
}

/// extras.image_to_bed_matrix: image pixel → bed mm from the alignment (target points, or the 4 corners).
pub fn image_to_bed(cal: &Value, ws: (f64, f64)) -> Option<H3> {
    let p2 = |v: &Value| -> Option<[f64; 2]> { Some([v.get(0)?.as_f64()?, v.get(1)?.as_f64()?]) };
    if let Some(pts) = cal.get("points").and_then(|v| v.as_array()).filter(|a| a.len() >= 4) {
        let (src, dst): (Vec<[f64; 2]>, Vec<[f64; 2]>) = pts.iter().filter_map(|p| Some((p2(p.get(0)?)?, p2(p.get(1)?)?))).unzip();
        return homography(&src, &dst);
    }
    if let Some(c) = cal.get("corners").and_then(|v| v.as_array()).filter(|a| a.len() == 4) {
        let src: Vec<[f64; 2]> = c.iter().filter_map(p2).collect();
        let (w, h) = ws;
        return homography(&src, &[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]);
    }
    None
}

fn bicubic(img: &RgbImage, x: f64, y: f64) -> Rgb<u8> {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (x, y) = (x - 0.5, y - 0.5);
    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
    if ix < -1 || iy < -1 || ix > w || iy > h {
        return Rgb([255, 255, 255]);
    }
    let (fx, fy) = (x - ix as f64, y - iy as f64);
    let k = |t: f64| {
        let a = -0.5;
        let t = t.abs();
        if t <= 1.0 { (a + 2.0) * t * t * t - (a + 3.0) * t * t + 1.0 } else if t < 2.0 { a * t * t * t - 5.0 * a * t * t + 8.0 * a * t - 4.0 * a } else { 0.0 }
    };
    let mut acc = [0.0f64; 3];
    for j in -1..=2 {
        for i in -1..=2 {
            let (px, py) = (ix + i, iy + j);
            let c = if px < 0 || py < 0 || px >= w || py >= h { [255, 255, 255] } else { img.get_pixel(px as u32, py as u32).0 };
            let wgt = k(i as f64 - fx) * k(j as f64 - fy);
            for ch in 0..3 {
                acc[ch] += c[ch] as f64 * wgt;
            }
        }
    }
    Rgb([acc[0].round().clamp(0.0, 255.0) as u8, acc[1].round().clamp(0.0, 255.0) as u8, acc[2].round().clamp(0.0, 255.0) as u8])
}

/// extras.warp_to_workspace: frame → top-down image of the work area (lens correction, then perspective).
pub fn warp_to_workspace(frame: &RgbImage, cal: &Value, ws: (f64, f64), max_px: f64) -> RgbImage {
    let (w, h) = ws;
    let scale = max_px / w.max(h);
    let (ow, oh) = ((w * scale).round().max(1.0) as u32, (h * scale).round().max(1.0) as u32);
    let k = |key: &str| cal.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let img = undistort(frame, k("k1"), k("k2"));
    match image_to_bed(cal, ws).and_then(|m| invert(&m)) {
        Some(inv) => {
            let mut out = RgbImage::new(ow, oh);
            for (x, y, p) in out.enumerate_pixels_mut() {
                // output pixel centre → bed mm → image pixel
                let q = apply_h(&inv, (x as f64 + 0.5) / scale, (y as f64 + 0.5) / scale);
                *p = bicubic(&img, q[0], q[1]);
            }
            out
        }
        None => image::imageops::resize(&img, ow, oh, image::imageops::FilterType::Lanczos3),
    }
}

/// extras.detect_mark: centre (mm) of the darkest blob near (x, y) in a bed-aligned overlay.
pub fn detect_mark(overlay: &RgbImage, ws: (f64, f64), x_mm: f64, y_mm: f64, search_mm: f64) -> Option<[f64; 2]> {
    let (w, h) = (overlay.width() as f64, overlay.height() as f64);
    let (sx, sy) = (w / ws.0, h / ws.1);
    let (cx, cy, r) = (x_mm * sx, y_mm * sy, search_mm * sx);
    let (x0, x1) = ((cx - r).max(0.0) as usize, (cx + r).min(w) as usize);
    let (y0, y1) = ((cy - r).max(0.0) as usize, (cy + r).min(h) as usize);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let grey = |x: usize, y: usize| { let p = overlay.get_pixel(x as u32, y as u32).0; (p[0] as f64 * 299.0 + p[1] as f64 * 587.0 + p[2] as f64 * 114.0) / 1000.0 };
    let vals: Vec<(usize, usize, f64)> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| (x, y, grey(x, y).round())).collect();
    let mean = vals.iter().map(|v| v.2).sum::<f64>() / vals.len() as f64;
    let min = vals.iter().map(|v| v.2).fold(f64::MAX, f64::min);
    let thr = (mean - 40.0).min((min + mean) / 2.0);
    let dark: Vec<&(usize, usize, f64)> = vals.iter().filter(|v| v.2 < thr).collect();
    if dark.len() < 5 {
        return None;
    }
    let (mut sw, mut sxw, mut syw) = (0.0, 0.0, 0.0);
    for (x, y, g) in dark {
        let wt = thr - g;
        sw += wt;
        sxw += (*x - x0) as f64 * wt;
        syw += (*y - y0) as f64 * wt;
    }
    Some([(x0 as f64 + sxw / sw + 0.5) / sx, (y0 as f64 + syw / sw + 0.5) / sy])
}

/// extras.fit_k1: radial k1 that best straightens a line clicked in the raw image.
pub fn fit_k1(points: &[[f64; 2]], size: (f64, f64)) -> f64 {
    let (w, h) = size;
    let (cx, cy, norm) = ((w - 1.0) / 2.0, (h - 1.0) / 2.0, (w / 2.0).hypot(h / 2.0));
    let mut best = (f64::INFINITY, 0.0);
    for i in 0..241 {
        let k1 = -0.6 + 1.2 * i as f64 / 240.0;
        let q: Vec<[f64; 2]> = points.iter().map(|p| {
            let n = [(p[0] - cx) / norm, (p[1] - cy) / norm];
            let mut qq = n;
            for _ in 0..6 {
                let r2 = qq[0] * qq[0] + qq[1] * qq[1];
                qq = [n[0] / (1.0 + k1 * r2), n[1] / (1.0 + k1 * r2)];
            }
            [qq[0] * norm + cx, qq[1] * norm + cy]
        }).collect();
        let m = [q.iter().map(|p| p[0]).sum::<f64>() / q.len() as f64, q.iter().map(|p| p[1]).sum::<f64>() / q.len() as f64];
        let (mut a, mut b, mut c) = (0.0, 0.0, 0.0);
        for p in &q {
            let (dx, dy) = (p[0] - m[0], p[1] - m[1]);
            a += dx * dx;
            b += dx * dy;
            c += dy * dy;
        }
        // smallest singular value of the centred points = sqrt(smallest eigenvalue of the 2×2 scatter)
        let tr = a + c;
        let det = a * c - b * b;
        let ev = (tr / 2.0 - ((tr * tr / 4.0 - det).max(0.0)).sqrt()).max(0.0);
        let resid = ev.sqrt();
        if resid < best.0 {
            best = (resid, k1);
        }
    }
    best.1
}

pub fn alignment_targets(ws: (f64, f64), n: usize, margin: f64) -> Vec<[f64; 2]> {
    let xs: Vec<f64> = (0..n).map(|i| ws.0 * (margin + (1.0 - 2.0 * margin) * i as f64 / (n - 1) as f64)).collect();
    let ys: Vec<f64> = (0..n).map(|j| ws.1 * (margin + (1.0 - 2.0 * margin) * j as f64 / (n - 1) as f64)).collect();
    ys.iter().flat_map(|y| xs.iter().map(move |x| [*x, *y])).collect()
}

// ====================================================================== capture
/// A running camera: the newest frame, filled by a capture thread.
pub struct Capture {
    pub frame: Arc<Mutex<Option<RgbImage>>>,
    pub error: Arc<Mutex<Option<String>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Camera names (index order).
pub fn list_cameras() -> Vec<String> {
    nokhwa::query(nokhwa::utils::ApiBackend::Auto).map(|v| v.iter().map(|c| c.human_name()).collect()).unwrap_or_default()
}

impl Capture {
    pub fn start(index: u32) -> Capture {
        let frame = Arc::new(Mutex::new(None));
        let error = Arc::new(Mutex::new(None));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (f, e, s) = (frame.clone(), error.clone(), stop.clone());
        std::thread::spawn(move || {
            use nokhwa::pixel_format::RgbFormat;
            use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
            let req = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestResolution);
            let mut cam = match nokhwa::Camera::new(CameraIndex::Index(index), req) {
                Ok(c) => c,
                Err(err) => {
                    *e.lock().unwrap() = Some(err.to_string());
                    return;
                }
            };
            if let Err(err) = cam.open_stream() {
                *e.lock().unwrap() = Some(err.to_string());
                return;
            }
            while !s.load(std::sync::atomic::Ordering::SeqCst) {
                match cam.frame().and_then(|b| b.decode_image::<RgbFormat>()) {
                    Ok(img) => {
                        if let Some(im) = RgbImage::from_raw(img.width(), img.height(), img.into_raw()) {
                            *f.lock().unwrap() = Some(im);
                        }
                    }
                    Err(err) => {
                        *e.lock().unwrap() = Some(err.to_string());
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                }
            }
            let _ = cam.stop_stream();
        });
        Capture { frame, error, stop }
    }
}
