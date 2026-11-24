//! Estimated time left for a scan (the counterpart of `realhires/eta.py`).
//!
//! Counts bytes rather than files: analysing a file takes time in proportion to
//! its audio.  Files answered from saved results take no time and are left
//! out, and parallel workers are covered because the throughput is measured
//! for all of them together.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub const MIN_FILES: usize = 3;
pub const MIN_SECONDS: f64 = 3.0;

/// {path: size}; an unreadable file counts as one byte.
pub fn file_sizes<'a>(paths: impl IntoIterator<Item = &'a String>) -> HashMap<String, u64> {
    paths.into_iter().map(|p| (p.clone(), std::fs::metadata(p).map(|m| m.len().max(1)).unwrap_or(1))).collect()
}

pub struct ScanEta {
    sizes: HashMap<String, u64>,
    total: u64,
    start: Instant,
    /// seconds since `start`; replaced by the tests
    pub clock: Box<dyn Fn(Instant) -> f64 + Send>,
    done_bytes: u64,
    done: HashSet<String>,
}

impl ScanEta {
    pub fn new(sizes: HashMap<String, u64>) -> Self {
        let total = sizes.values().sum();
        ScanEta {
            sizes,
            total,
            start: Instant::now(),
            clock: Box::new(|s| s.elapsed().as_secs_f64()),
            done_bytes: 0,
            done: HashSet::new(),
        }
    }

    /// One file has been analysed (repeats and unknown paths are ignored).
    pub fn add(&mut self, path: &str) {
        if let Some(n) = self.sizes.get(path) {
            if self.done.insert(path.to_string()) {
                self.done_bytes += n;
            }
        }
    }

    pub fn seconds_left(&self) -> Option<f64> {
        let elapsed = (self.clock)(self.start);
        if self.done.len() < MIN_FILES.min(self.sizes.len()) || elapsed < MIN_SECONDS || self.done_bytes == 0 {
            return None;
        }
        Some(((self.total - self.done_bytes) as f64 * elapsed / self.done_bytes as f64).max(0.0))
    }

    pub fn text(&self) -> String {
        fmt_left(self.seconds_left())
    }
}

/// "about 6 min left", "about 1 h 20 min left", "less than a minute left".
pub fn fmt_left(seconds: Option<f64>) -> String {
    match seconds {
        None => "estimating the time left".into(),
        Some(s) => format!("{} left", fmt_about(s)),
    }
}

/// "about 6 min", "about 1 h 20 min", "less than a minute".
pub fn fmt_about(s: f64) -> String {
    if s < 60.0 {
        return "less than a minute".into();
    }
    let minutes = (s / 60.0).floor() as u64 + if s % 60.0 >= 30.0 { 1 } else { 0 };
    if minutes < 60 {
        return format!("about {minutes} min");
    }
    let (h, m) = (minutes / 60, minutes % 60);
    if m > 0 {
        format!("about {h} h {m} min")
    } else {
        format!("about {h} h")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn counts_bytes_not_files() {
        let sizes: HashMap<String, u64> =
            [("a", 100), ("b", 100), ("c", 100), ("d", 700)].iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let t = Arc::new(Mutex::new(0.0));
        let mut eta = ScanEta::new(sizes);
        let tc = t.clone();
        eta.clock = Box::new(move |_| *tc.lock().unwrap());
        assert_eq!(eta.text(), "estimating the time left");
        for p in ["a", "b", "c", "c", "zzz"] {
            eta.add(p);
        }
        *t.lock().unwrap() = 30.0;
        assert_eq!(eta.seconds_left(), Some(70.0));
        *t.lock().unwrap() = 300.0;
        assert_eq!(eta.text(), "about 12 min left");
        assert_eq!(fmt_left(Some(3600.0 * 1.5)), "about 1 h 30 min left");
        assert_eq!(fmt_left(Some(3600.0)), "about 1 h left");
    }
}
