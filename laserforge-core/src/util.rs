//! Number formatting that matches Python's, so the G-code is identical to the Python engine's.

/// Python `f"{v:.3f}"`.
pub fn f3(v: f64) -> String {
    format!("{:.3}", v)
}

/// Python `f"{v:.1f}"`.
pub fn f1(v: f64) -> String {
    format!("{:.1}", v)
}

/// Python `f"{v:.0f}"`.
pub fn f0(v: f64) -> String {
    format!("{:.0}", v)
}

/// Python `round(v, n)` for floats (correctly rounded, ties to even on the exact value).
pub fn py_round(v: f64, n: usize) -> f64 {
    let k = 10f64.powi(n as i32);
    let scaled = v * k;
    let frac = scaled - scaled.floor();
    if (frac - 0.5).abs() < 1e-6 || !scaled.is_finite() {
        // near a tie: decide on the exact decimal value, as Python does
        return format!("{:.*}", n, v).parse().unwrap_or(v);
    }
    let r = scaled.round() / k;
    // dividing back can land one ulp off the decimal Python would print: snap through the exact path when unsure
    if (r * k - scaled.round()).abs() > 1e-9 {
        return format!("{:.*}", n, v).parse().unwrap_or(v);
    }
    r
}

/// Python `str(float)` / `repr(float)` for ordinary magnitudes: shortest round-trip digits, always a dot.
pub fn py_float(v: f64) -> String {
    if !v.is_finite() {
        return if v.is_nan() { "nan".into() } else if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let s = format!("{}", v);
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}
