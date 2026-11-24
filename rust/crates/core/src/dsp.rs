//! The NumPy functions the detection rules use, with NumPy's exact semantics
//! (median, linear percentile, `convolve(mode="same")`, `polyfit(deg=1)`,
//! window functions), so thresholds tuned on the Python version hold here.

/// `numpy.median` (NaN for an empty slice).
pub fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// `numpy.percentile(v, q)` with the default linear method.
pub fn percentile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    percentile_sorted(&s, q)
}

pub fn percentile_sorted(s: &[f64], q: f64) -> f64 {
    let n = s.len();
    let q = q / 100.0;
    // numpy: virtual index = n*q + (alpha + q*(1 - alpha - beta)) - 1 with alpha = beta = 1
    let (alpha, beta) = (1.0, 1.0);
    let vi = n as f64 * q + (alpha + q * (1.0 - alpha - beta)) - 1.0;
    let prev = vi.floor().clamp(0.0, (n - 1) as f64);
    let next = (prev + 1.0).min((n - 1) as f64);
    let gamma = vi - prev;
    let (a, b) = (s[prev as usize], s[next as usize]);
    // numpy's _lerp
    let diff = b - a;
    if gamma >= 0.5 {
        b - diff * (1.0 - gamma)
    } else {
        a + diff * gamma
    }
}

/// `numpy.convolve(a, ones(k)/k, mode="same")` for odd k (direct sums, no
/// running total: the spectrum spans 200 dB and a running sum would bury
/// quiet bins under the rounding error of loud ones).
pub fn moving_average_same(a: &[f64], k: usize) -> Vec<f64> {
    let n = a.len();
    let w = 1.0 / k as f64;
    let half = (k - 1) / 2;
    let mut out = vec![0.0; n];
    for (i, o) in out.iter_mut().enumerate() {
        let lo = i.saturating_sub(half);
        let hi = (i + half).min(n - 1);
        let mut s = 0.0;
        for x in &a[lo..=hi] {
            s += x * w;
        }
        *o = s;
    }
    out
}

/// Least-squares slope of y over x (`numpy.polyfit(x, y, 1)[0]`).
pub fn slope(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len() as f64;
    let mx = x.iter().sum::<f64>() / n;
    let my = y.iter().sum::<f64>() / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
    }
    sxy / sxx
}

/// `numpy.hanning(n)`.
pub fn hanning(n: usize) -> Vec<f64> {
    if n == 1 {
        return vec![1.0];
    }
    let m = (n - 1) as f64;
    (0..n)
        .map(|i| {
            let k = 1.0 - n as f64 + 2.0 * i as f64;
            0.5 + 0.5 * (std::f64::consts::PI * k / m).cos()
        })
        .collect()
}

/// 4-term Blackman-Harris window (periodic, about -92 dB side lobes).
pub fn blackman_harris(n: usize) -> Vec<f64> {
    (0..n)
        .map(|k| {
            let x = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
            0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos()
        })
        .collect()
}

/// `numpy.linspace(start, stop, num)`.
pub fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    if num == 1 {
        return vec![start];
    }
    let step = (stop - start) / (num - 1) as f64;
    let mut v: Vec<f64> = (0..num).map(|i| i as f64 * step + start).collect();
    v[num - 1] = stop;
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numpy_semantics() {
        assert_eq!(median(&[3.0, 1.0, 2.0, 10.0]), 2.5);
        assert!((percentile(&[1.0, 2.0, 3.0, 4.0, 5.0], 10.0) - 1.4).abs() < 1e-12);
        assert!((percentile(&[1.0, 2.0, 3.0, 4.0], 25.0) - 1.75).abs() < 1e-12);
        let ma = moving_average_same(&[1.0, 2.0, 3.0, 4.0, 5.0], 3);
        assert!((ma[0] - 1.0).abs() < 1e-12 && (ma[2] - 3.0).abs() < 1e-12 && (ma[4] - 3.0).abs() < 1e-12);
        assert!((slope(&[0.0, 1.0, 2.0], &[1.0, 3.0, 5.0]) - 2.0).abs() < 1e-12);
        let h = hanning(5);
        assert!((h[0]).abs() < 1e-12 && (h[2] - 1.0).abs() < 1e-12);
    }
}
