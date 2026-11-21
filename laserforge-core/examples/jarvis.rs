use laserforge_core::imaging::{dither, Adj, Grey};
fn main() {
    let (c, r) = (3000usize, 3000usize);
    let v: Vec<f32> = (0..c * r).map(|i| ((i % c) as f32 * 0.085 + (i / c) as f32 * 0.03) % 255.0).collect();
    let g = Grey { cols: c, rows: r, v };
    let adj = Adj { mode: "jarvis".into(), ..Default::default() };
    let t = std::time::Instant::now();
    let out = dither(&g, &adj, 0.1);
    println!("rust jarvis 3000x3000: {:.2}s ({} burned px)", t.elapsed().as_secs_f64(), out.iter().filter(|&&b| b > 0).count());
}
