//! Number formatting and rounding exactly as Python does it.
//!
//! Every reason, summary and table cell of Real-Hi-Res is the same text in the
//! Python and the Rust version.  Rust's fixed-precision formatting (`{:.1}`)
//! already rounds like Python's (`{:.1f}`); this module adds what Rust lacks:
//! `{:g}`, `{:%}`, `repr(float)`, `round(x, n)` and Python's "nan"/"inf".

/// `f"{x:.{prec}f}"`.
pub fn f(x: f64, prec: usize) -> String {
    if let Some(s) = special(x) {
        return s;
    }
    format!("{x:.prec$}")
}

/// `f"{x:+.{prec}f}"`.
pub fn fs(x: f64, prec: usize) -> String {
    if let Some(s) = special(x) {
        return if x.is_nan() || x < 0.0 { s } else { format!("+{s}") };
    }
    format!("{x:+.prec$}")
}

/// `f"{x:.{prec}%}"`.
pub fn pct(x: f64, prec: usize) -> String {
    format!("{}%", f(x * 100.0, prec))
}

fn special(x: f64) -> Option<String> {
    if x.is_nan() {
        Some("nan".into())
    } else if x == f64::INFINITY {
        Some("inf".into())
    } else if x == f64::NEG_INFINITY {
        Some("-inf".into())
    } else {
        None
    }
}

/// Shortest round-trip digits and decimal exponent: x = 0.d1d2d3... × 10^decpt.
fn shortest_digits(x: f64) -> (bool, String, i32) {
    let s = format!("{:e}", x.abs());
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let exp: i32 = exp.parse().unwrap_or(0);
    (x.is_sign_negative(), digits, exp + 1)
}

/// `repr(x)` of a Python float ("25.0", "0.975", "1e-05", "1e+16").
pub fn repr_float(x: f64) -> String {
    if let Some(s) = special(x) {
        return s;
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let (neg, digits, decpt) = shortest_digits(x);
    let sign = if neg { "-" } else { "" };
    if decpt > -4 && decpt <= 16 {
        let body = if decpt <= 0 {
            format!("0.{}{}", "0".repeat((-decpt) as usize), digits)
        } else if decpt as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(decpt as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..decpt as usize], &digits[decpt as usize..])
        };
        format!("{sign}{body}")
    } else {
        let mant = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits.clone() };
        let e = decpt - 1;
        format!("{sign}{mant}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    }
}

/// `f"{x:g}"` (6 significant digits, trailing zeros removed).
pub fn g(x: f64) -> String {
    if let Some(s) = special(x) {
        return s;
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let sci = format!("{:.5e}", x);
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    if (-4..6).contains(&exp) {
        let prec = (5 - exp).max(0) as usize;
        strip_zeros(&format!("{x:.prec$}"))
    } else {
        let m = strip_zeros(mant);
        format!("{m}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
    }
}

fn strip_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// `round(x, n)` for a float (correctly rounded, like Python).
pub fn round_to(x: f64, n: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.n$}").parse().unwrap_or(x)
}

/// `int(round(x))`: rounds half to even.
pub fn round_int(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// `numpy.round(x, n)` (scale, round half to even, scale back).
pub fn np_round(x: f64, n: i32) -> f64 {
    let s = 10f64.powi(n);
    (x * s).round_ties_even() / s
}

/// Python's `str.ljust`/`{:<n}` and `rjust` count characters, as Rust does.
pub fn rjust(s: &str, n: usize) -> String {
    format!("{s:>n$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_like_python() {
        for (x, want) in [
            (25.0, "25.0"),
            (0.975, "0.975"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (123456.789, "123456.789"),
            (-0.5, "-0.5"),
            (1.5e300, "1.5e+300"),
            (0.1 + 0.2, "0.30000000000000004"),
        ] {
            assert_eq!(repr_float(x), want);
        }
    }

    #[test]
    fn g_like_python() {
        for (x, want) in [
            (44.1, "44.1"),
            (22.05, "22.05"),
            (48.0, "48"),
            (1e-5, "1e-05"),
            (1e16, "1e+16"),
            (123456.0, "123456"),
            (1234567.0, "1.23457e+06"),
            (0.0001234, "0.0001234"),
            (-0.0, "-0"),
        ] {
            assert_eq!(g(x), want, "{x}");
        }
    }

    #[test]
    fn round_and_percent() {
        assert_eq!(round_to(2.675, 2), 2.67); // 2.675 is just below the tie in binary
        assert_eq!(round_to(0.85634, 4), 0.8563);
        assert_eq!(pct(0.856, 0), "86%");
        assert_eq!(pct(0.5003, 1), "50.0%");
        assert_eq!(fs(-2.5, 1), "-2.5");
        assert_eq!(fs(3.04, 1), "+3.0");
        assert_eq!(f(f64::NAN, 0), "nan");
        assert_eq!(round_int(2.5), 2);
        assert_eq!(round_int(3.5), 4);
    }
}
