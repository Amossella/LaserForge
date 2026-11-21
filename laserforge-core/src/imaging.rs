//! Image processing for engraving: adjustments, dithering, rotation to the scan angle, decoding.
//! Ported from LaserForge's Python (laserforge/model.py and the raster rotation in gcode.py).
//! Brightness/contrast/gamma run in f32 like the numpy code; error diffusion runs in f64 like Python floats.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Adj {
    pub mode: String,
    pub brightness: f64,
    pub contrast: f64,
    pub gamma: f64,
    pub invert: bool,
    pub threshold: f64,
    pub cells_per_inch: f64,
    pub halftone_angle: f64,
    pub slices: i64,
    pub enhance_radius: f64,
    pub enhance_amount: f64,
    pub enhance_denoise: f64,
}

impl Default for Adj {
    fn default() -> Self {
        Adj {
            mode: "dither".into(),
            brightness: 0.0,
            contrast: 0.0,
            gamma: 1.0,
            invert: false,
            threshold: 128.0,
            cells_per_inch: 30.0,
            halftone_angle: 22.5,
            slices: 10,
            enhance_radius: 0.0,
            enhance_amount: 0.0,
            enhance_denoise: 0.0,
        }
    }
}

/// A grey image: values 0..255 (255 = white), row-major.
pub struct Grey {
    pub cols: usize,
    pub rows: usize,
    pub v: Vec<f32>,
}

/// Brightness, contrast, gamma and invert, in the same order and precision as the Python code.
pub fn adjust(g: &mut Grey, adj: &Adj) {
    let c = adj.contrast * 2.55;
    let cf = ((259.0 * (c + 255.0)) / (255.0 * (259.0 - c))) as f32;
    let b = (adj.brightness * 2.55) as f32;
    for v in g.v.iter_mut() {
        *v = (cf * (*v - 128.0) + 128.0 + b).clamp(0.0, 255.0);
    }
    let gamma = if adj.gamma != 0.0 { adj.gamma } else { 1.0 };
    if (gamma - 1.0).abs() > 1e-3 {
        let e = (1.0 / gamma) as f32;
        for v in g.v.iter_mut() {
            *v = 255.0f32 * (*v / 255.0f32).powf(e);
        }
    }
    if adj.invert {
        for v in g.v.iter_mut() {
            *v = 255.0 - *v;
        }
    }
    for v in g.v.iter_mut() {
        *v = v.clamp(0.0, 255.0);
    }
}

const JARVIS: (&[(i32, i32, f64)], f64) = (&[(1, 0, 7.), (2, 0, 5.), (-2, 1, 3.), (-1, 1, 5.), (0, 1, 7.), (1, 1, 5.), (2, 1, 3.),
    (-2, 2, 1.), (-1, 2, 3.), (0, 2, 5.), (1, 2, 3.), (2, 2, 1.)], 48.);
const STUCKI: (&[(i32, i32, f64)], f64) = (&[(1, 0, 8.), (2, 0, 4.), (-2, 1, 2.), (-1, 1, 4.), (0, 1, 8.), (1, 1, 4.), (2, 1, 2.),
    (-2, 2, 1.), (-1, 2, 2.), (0, 2, 4.), (1, 2, 2.), (2, 2, 1.)], 42.);
const ATKINSON: (&[(i32, i32, f64)], f64) = (&[(1, 0, 1.), (2, 0, 1.), (-1, 1, 1.), (0, 1, 1.), (1, 1, 1.), (0, 2, 1.)], 8.);
const SIERRA: (&[(i32, i32, f64)], f64) = (&[(1, 0, 5.), (2, 0, 3.), (-2, 1, 2.), (-1, 1, 4.), (0, 1, 5.), (1, 1, 4.), (2, 1, 2.),
    (-1, 2, 2.), (0, 2, 3.), (1, 2, 2.)], 32.);

const BAYER8: [[f32; 8]; 8] = [[0., 32., 8., 40., 2., 34., 10., 42.], [48., 16., 56., 24., 50., 18., 58., 26.],
    [12., 44., 4., 36., 14., 46., 6., 38.], [60., 28., 52., 20., 62., 30., 54., 22.], [3., 35., 11., 43., 1., 33., 9., 41.],
    [51., 19., 59., 27., 49., 17., 57., 25.], [15., 47., 7., 39., 13., 45., 5., 37.], [63., 31., 55., 23., 61., 29., 53., 21.]];

/// Serpentine error diffusion (Python float arithmetic). Returns burn 0/255.
fn error_diffuse(g: &Grey, kernel: &[(i32, i32, f64)], div: f64) -> Vec<u8> {
    let (w, h) = (g.cols as i64, g.rows as i64);
    let mut buf: Vec<f64> = g.v.iter().map(|&v| v as f64).collect();
    let mut out = vec![0u8; g.v.len()];
    for y in 0..h {
        let rev = y % 2 == 1;
        let sign = if rev { -1 } else { 1 };
        for i in 0..w {
            let x = if rev { w - 1 - i } else { i };
            let idx = (y * w + x) as usize;
            let old = buf[idx];
            let new = if old >= 128.0 { 255.0 } else { 0.0 };
            if new == 0.0 {
                out[idx] = 255;
            }
            let err = old - new;
            if err == 0.0 {
                continue;
            }
            for &(dx, dy, wgt) in kernel {
                let xx = x + dx as i64 * sign;
                let yy = y + dy as i64;
                if xx >= 0 && xx < w && yy < h {
                    buf[(yy * w + xx) as usize] += err * wgt / div;
                }
            }
        }
    }
    out
}

/// Pillow's Floyd–Steinberg (integer arithmetic, left to right), used for the default "dither" mode.
fn floyd_steinberg(g: &Grey) -> Vec<u8> {
    let (w, h) = (g.cols, g.rows);
    let mut out = vec![0u8; w * h];
    let mut errors = vec![0i32; w + 1];
    for y in 0..h {
        let (mut l, mut l0, mut l1) = (0i32, 0i32, 0i32);
        for x in 0..w {
            let px = g.v[y * w + x] as i32; // the image is converted to 8 bits first (truncation)
            l = (px + (l + errors[x + 1]) / 16).clamp(0, 255);
            let white = l > 128;
            out[y * w + x] = if white { 0 } else { 255 };
            l -= if white { 255 } else { 0 };
            let l2 = l;
            let d2 = l + l;
            l += d2;
            errors[x] = l + l0;
            l += d2;
            l0 = l + l1;
            l1 = l2;
            l += d2;
        }
        errors[w] = l0;
    }
    out
}

fn halftone(g: &Grey, px_mm: f64, cell_mm: f64, angle_deg: f64) -> Vec<u8> {
    let t = angle_deg.to_radians();
    let (ct, st) = (t.cos() as f32, t.sin() as f32);
    let (px, cell) = (px_mm as f32, cell_mm as f32);
    let mut out = vec![0u8; g.v.len()];
    for y in 0..g.rows {
        for x in 0..g.cols {
            let xx = x as f32 * px;
            let yy = y as f32 * px;
            let u = (xx * ct + yy * st) / cell;
            let v = (-xx * st + yy * ct) / cell;
            let (du, dv) = (u - u.floor() - 0.5, v - v.floor() - 0.5);
            let dist = (du * du + dv * dv).sqrt();
            let darkness = 1.0f32 - g.v[y * g.cols + x] / 255.0f32;
            let mut radius = (darkness / std::f32::consts::PI).sqrt();
            if darkness > 0.785f32 {
                radius = darkness.sqrt() * 0.7071f32 + 0.0f32;
            }
            out[y * g.cols + x] = if dist < radius { 255 } else { 0 };
        }
    }
    out
}

fn sketch(g: &Grey, threshold: f64) -> Vec<u8> {
    let (w, h) = (g.cols, g.rows);
    let at = |x: usize, y: usize| g.v[y * w + x];
    let grad = |a: f32, b: f32, two: bool| if two { (b - a) / 2.0 } else { b - a };
    let lim = ((256.0 - threshold) / 6.0) as f32;
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let gy = if h < 2 { 0.0 } else if y == 0 { grad(at(x, 0), at(x, 1), false) } else if y == h - 1 { grad(at(x, h - 2), at(x, h - 1), false) } else { grad(at(x, y - 1), at(x, y + 1), true) };
            let gx = if w < 2 { 0.0 } else if x == 0 { grad(at(0, y), at(1, y), false) } else if x == w - 1 { grad(at(w - 2, y), at(w - 1, y), false) } else { grad(at(x - 1, y), at(x + 1, y), true) };
            out[y * w + x] = if gx.hypot(gy) > lim { 255 } else { 0 };
        }
    }
    out
}

/// Grey (255 = white) → burn values 0..255 with the chosen method.
pub fn dither(g: &Grey, adj: &Adj, px_mm: f64) -> Vec<u8> {
    match adj.mode.as_str() {
        "threshold" => g.v.iter().map(|&v| if (v as f64) < adj.threshold { 255 } else { 0 }).collect(),
        "gray" | "slices3d" => g.v.iter().map(|&v| (255.0 - (v.round_ties_even())) as u8).collect(),
        "jarvis" => error_diffuse(g, JARVIS.0, JARVIS.1),
        "stucki" => error_diffuse(g, STUCKI.0, STUCKI.1),
        "atkinson" => error_diffuse(g, ATKINSON.0, ATKINSON.1),
        "sierra" => error_diffuse(g, SIERRA.0, SIERRA.1),
        "bayer" => (0..g.v.len()).map(|i| {
            let (x, y) = (i % g.cols, i / g.cols);
            let thr = (BAYER8[y % 8][x % 8] + 0.5f32) * 4.0f32;
            if g.v[i] < thr { 255 } else { 0 }
        }).collect(),
        "halftone" | "newsprint" => {
            // like the Python code: the "cells per inch" setting applies to both (50 is only a missing-key fallback)
            let cpi = adj.cells_per_inch;
            let cell = 25.4 / cpi.max(1.0);
            let angle = if adj.mode == "halftone" { adj.halftone_angle } else { 45.0 };
            halftone(g, px_mm, cell, angle)
        }
        "sketch" => sketch(g, adj.threshold),
        _ => floyd_steinberg(g),
    }
}

/// Rotate a burn bitmap counter-clockwise (in image coordinates) with nearest-neighbour sampling and an
/// expanded canvas filled with 0, following Pillow's `rotate(angle, NEAREST, expand=True)`.
pub fn rotate_nearest(data: &[u8], cols: usize, rows: usize, angle_deg: f64) -> (Vec<u8>, usize, usize) {
    let angle = -(angle_deg.rem_euclid(360.0)).to_radians();
    let r15 = |v: f64| (v * 1e15).round() / 1e15;
    let (w, h) = (cols as f64, rows as f64);
    let mut m = [r15(angle.cos()), r15(angle.sin()), 0.0, r15(-angle.sin()), r15(angle.cos()), 0.0];
    let tf = |m: &[f64; 6], x: f64, y: f64| (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (a, b) = tf(&m, -cx, -cy);
    m[2] = a + cx;
    m[5] = b + cy;
    let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| tf(&m, x, y));
    let nw = (corners.iter().map(|c| c.0).fold(f64::MIN, f64::max).ceil() - corners.iter().map(|c| c.0).fold(f64::MAX, f64::min).floor()) as usize;
    let nh = (corners.iter().map(|c| c.1).fold(f64::MIN, f64::max).ceil() - corners.iter().map(|c| c.1).fold(f64::MAX, f64::min).floor()) as usize;
    let (a, b) = tf(&m, -((nw as f64) - w) / 2.0, -((nh as f64) - h) / 2.0);
    m[2] = a;
    m[5] = b;
    let mut out = vec![0u8; nw * nh];
    for y in 0..nh {
        for x in 0..nw {
            let (xi, yi) = tf(&m, x as f64 + 0.5, y as f64 + 0.5);
            if xi >= 0.0 && yi >= 0.0 {
                let (ix, iy) = (xi as usize, yi as usize);
                if ix < cols && iy < rows {
                    out[y * nw + x] = data[iy * cols + ix];
                }
            }
        }
    }
    (out, nw, nh)
}

/// Pillow's NEAREST resize of a burn bitmap.
pub fn resize_nearest(data: &[u8], cols: usize, rows: usize, nc: usize, nr: usize) -> Vec<u8> {
    let (sx, sy) = (cols as f64 / nc as f64, rows as f64 / nr as f64);
    let mut out = vec![0u8; nc * nr];
    for y in 0..nr {
        let iy = (((y as f64 + 0.5) * sy) as usize).min(rows - 1);
        for x in 0..nc {
            let ix = (((x as f64 + 0.5) * sx) as usize).min(cols - 1);
            out[y * nc + x] = data[iy * cols + ix];
        }
    }
    out
}

/// A decoded picture: grey (on white where transparent) and its alpha when it has transparency.
pub struct Picture {
    pub img: image::GrayImage,
    pub alpha: Option<image::GrayImage>,
}

/// Decode a `data:image/...;base64,` URL or raw bytes (PNG, JPEG, BMP, GIF, WebP).
pub fn decode(src: &str) -> Option<Picture> {
    use base64::Engine;
    let b64 = src.split_once(',').map(|x| x.1).unwrap_or(src);
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    decode_bytes(&bytes)
}

pub fn decode_bytes(bytes: &[u8]) -> Option<Picture> {
    let dynimg = image::load_from_memory(bytes).ok()?;
    let rgba = dynimg.to_rgba8();
    let has_alpha = rgba.pixels().any(|p| p.0[3] < 255);
    let (w, h) = rgba.dimensions();
    let mut grey = image::GrayImage::new(w, h);
    let mut alpha = image::GrayImage::new(w, h);
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = p.0[3] as f32 / 255.0;
        // composite on white, then ITU-R 601 luma like PIL's "L" conversion
        let c = |v: u8| v as f32 * a + 255.0 * (1.0 - a);
        let l = (c(p.0[0]) * 299.0 + c(p.0[1]) * 587.0 + c(p.0[2]) * 114.0) / 1000.0;
        grey.put_pixel(x, y, image::Luma([l.round().clamp(0.0, 255.0) as u8]));
        alpha.put_pixel(x, y, image::Luma([p.0[3]]));
    }
    Some(Picture { img: grey, alpha: if has_alpha { Some(alpha) } else { None } })
}

/// Whole pipeline for one image shape: resample to the line interval (Lanczos), sharpen, adjust, dither,
/// mask transparency. `pass_through` keeps the picture's own pixels. Returns (burn, cols, rows).
#[allow(clippy::too_many_arguments)]
pub fn process(pic: &Picture, w_mm: f64, h_mm: f64, interval: f64, max_px: usize, pass_through: bool,
               flip_x: bool, flip_y: bool, adj: &Adj) -> (Vec<u8>, usize, usize) {
    use image::imageops::{self, FilterType};
    let (iw, ih) = pic.img.dimensions();
    let (mut cols, mut rows) = if pass_through {
        (iw as f64, ih as f64)
    } else {
        ((w_mm / interval).round_ties_even().max(1.0), (h_mm / interval).round_ties_even().max(1.0))
    };
    let k = (max_px as f64 / cols.max(rows)).min(1.0);
    cols = (cols * k).round_ties_even().max(1.0);
    rows = (rows * k).round_ties_even().max(1.0);
    let (c, r) = (cols as u32, rows as u32);
    let mut g = if (c, r) != (iw, ih) {
        imageops::resize(&pic.img, c, r, if pass_through { FilterType::Nearest } else { FilterType::Lanczos3 })
    } else {
        pic.img.clone()
    };
    let mut alpha = pic.alpha.as_ref().map(|a| imageops::resize(a, c, r, FilterType::Triangle));
    if flip_x {
        g = imageops::flip_horizontal(&g);
        alpha = alpha.map(|a| imageops::flip_horizontal(&a));
    }
    if flip_y {
        g = imageops::flip_vertical(&g);
        alpha = alpha.map(|a| imageops::flip_vertical(&a));
    }
    let n = (c * r) as usize;
    let mut burn: Vec<u8>;
    if pass_through {
        let gray = adj.mode == "gray" || adj.mode == "slices3d";
        burn = g.pixels().map(|p| if gray { 255 - p.0[0] } else if p.0[0] < 128 { 255 } else { 0 }).collect();
    } else {
        if adj.enhance_amount > 0.0 && adj.enhance_radius > 0.0 {
            let radius_px = (adj.enhance_radius / (w_mm / cols).max(1e-6)) as f32;
            let blurred = imageops::blur(&g, radius_px.max(0.1));
            let (amount, thr) = (adj.enhance_amount as f32 / 100.0, adj.enhance_denoise as f32);
            for (p, b) in g.pixels_mut().zip(blurred.pixels()) {
                let d = p.0[0] as f32 - b.0[0] as f32;
                if d.abs() >= thr {
                    p.0[0] = (p.0[0] as f32 + d * amount).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        let mut grey = Grey { cols: c as usize, rows: r as usize, v: g.pixels().map(|p| p.0[0] as f32).collect() };
        adjust(&mut grey, adj);
        burn = dither(&grey, adj, w_mm / cols);
    }
    if let Some(a) = alpha {
        for (b, p) in burn.iter_mut().zip(a.pixels()) {
            if p.0[0] < 128 {
                *b = 0;
            }
        }
    }
    burn.truncate(n);
    (burn, c as usize, r as usize)
}
