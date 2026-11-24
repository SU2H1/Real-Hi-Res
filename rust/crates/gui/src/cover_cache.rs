//! The player's album covers as textures (`docs/player-spec.md`, "Album covers
//! are kept").  One worker thread gets them with `covers::cached_cover_png()`
//! (from the covers folder on disk, else made by FFmpeg and saved there for the
//! next time) and decodes them: first the covers asked for on screen, newest
//! first, then, after `prefetch()`, the grid-size covers of every album that the
//! covers folder does not have yet.  Textures are kept for the session (the
//! large size, when it is not the grid size, up to `MAX_LARGE`), and
//! `library_changed()` drops only those of albums whose first songs changed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};

use realhires::covers::{self, CACHE_PATHS};
use realhires::musiclib::Album;

/// The covers of the album grid, the album page, the mosaics and the LCD on a
/// normal screen (the size of the Python app's album page, so the two apps
/// share these files).
pub const GRID_PX: u32 = 220;
/// Now Playing, and everything on a high-density screen (the Python app's
/// largest Now Playing size).
pub const LARGE_PX: u32 = 480;
/// The widest album card, in points.
const CARD_POINTS: f32 = 240.0;
/// Large textures kept when they are not the grid size.
pub const MAX_LARGE: usize = 64;
/// Textures made per frame, so a burst of covers never stalls scrolling.
const PER_FRAME: usize = 24;

struct Job {
    key: String,
    size: u32,
    paths: Vec<String>,
    generation: u64,
}

#[derive(Default)]
struct Jobs {
    /// asked for on screen (served newest first)
    front: Vec<Job>,
    /// made ahead, in album order
    back: VecDeque<Job>,
    stop: bool,
}

type Shared = Arc<(Mutex<Jobs>, Condvar)>;
type Done = (String, u32, u64, Option<egui::ColorImage>);

pub struct Covers {
    jobs: Shared,
    rx: Receiver<Done>,
    /// album key -> (size, texture or None for "no cover")
    pub tex: HashMap<String, Vec<(u32, Option<egui::TextureHandle>)>>,
    pending: HashSet<(String, u32)>,
    /// when each large texture was last shown
    used: HashMap<(String, u32), u64>,
    tick: u64,
    /// album key -> its first songs (path, size, mtime)
    signatures: HashMap<String, Vec<(String, i64, i64)>>,
    /// album key -> the generation in which its covers were dropped (older answers are stale)
    dropped: HashMap<String, u64>,
    generation: u64,
    ctx: egui::Context,
}

fn decode(png: &[u8]) -> Option<egui::ColorImage> {
    let rgba = image::load_from_memory(png).ok()?.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

fn work(jobs: Shared, done: mpsc::Sender<Done>, dir: PathBuf, ctx: egui::Context) {
    let ffmpeg = realhires::probe::tool("ffmpeg");
    loop {
        let (job, shown) = {
            let (lock, cv) = &*jobs;
            let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
            while !q.stop && q.front.is_empty() && q.back.is_empty() {
                q = cv.wait(q).unwrap_or_else(|e| e.into_inner());
            }
            if q.stop {
                return;
            }
            match q.front.pop() {
                Some(j) => (j, true),
                None => (q.back.pop_front().expect("a job"), false),
            }
        };
        if !shown {
            // made ahead: only on disk, the screen asks for it when it is shown
            if !covers::is_cached_in(&dir, &job.paths, job.size) {
                let _ = covers::cached_cover_png_in(&dir, &ffmpeg, &job.paths, job.size);
            }
            continue;
        }
        let img = covers::cached_cover_png_in(&dir, &ffmpeg, &job.paths, job.size).and_then(|d| decode(&d));
        if done.send((job.key, job.size, job.generation, img)).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

impl Covers {
    pub fn new(ctx: &egui::Context) -> Covers {
        let jobs: Shared = Arc::new((Mutex::new(Jobs::default()), Condvar::new()));
        let (done, rx) = mpsc::channel();
        let dir = covers::cache_dir();
        let (j, c, d) = (jobs.clone(), ctx.clone(), dir.clone());
        std::thread::Builder::new()
            .name("player-covers".into())
            .spawn(move || work(j, done, d, c))
            .expect("cover thread");
        std::thread::Builder::new()
            .name("player-covers-trim".into())
            .spawn(move || covers::trim_cache(covers::CACHE_LIMIT_BYTES, Some(&dir)))
            .expect("cover trim thread");
        Covers {
            jobs,
            rx,
            tex: HashMap::new(),
            pending: HashSet::new(),
            used: HashMap::new(),
            tick: 0,
            signatures: HashMap::new(),
            dropped: HashMap::new(),
            generation: 0,
            ctx: ctx.clone(),
        }
    }

    /// The size of the grid covers on this screen: `GRID_PX`, or `LARGE_PX` when
    /// the widest card needs more pixels than that.
    pub fn grid_px(&self) -> u32 {
        if CARD_POINTS * self.ctx.pixels_per_point() <= GRID_PX as f32 * 1.2 {
            GRID_PX
        } else {
            LARGE_PX
        }
    }

    fn lookup(&self, key: &str, size: u32) -> Option<&Option<egui::TextureHandle>> {
        self.tex.get(key)?.iter().find(|(s, _)| *s == size).map(|(_, t)| t)
    }

    fn want(&mut self, key: &str, paths: &[String], size: u32) -> Option<egui::TextureHandle> {
        if let Some(t) = self.lookup(key, size) {
            let t = t.clone();
            if size > self.grid_px() {
                self.tick += 1;
                self.used.insert((key.to_string(), size), self.tick);
            }
            return t;
        }
        if self.pending.insert((key.to_string(), size)) {
            let job = Job {
                key: key.to_string(),
                size,
                paths: paths.iter().take(CACHE_PATHS).cloned().collect(),
                generation: self.generation,
            };
            let (lock, cv) = &*self.jobs;
            lock.lock().unwrap_or_else(|e| e.into_inner()).front.push(job);
            cv.notify_one();
        }
        // meanwhile another size of the same album, if there is one
        [LARGE_PX, GRID_PX].into_iter().filter(|s| *s != size).find_map(|s| self.lookup(key, s).cloned().flatten())
    }

    /// The cover of an album at the grid size (None while it is on its way, or
    /// when it has none): cards, the album page, mosaics, the LCD.
    pub fn get(&mut self, key: &str, paths: &[String]) -> Option<egui::TextureHandle> {
        let size = self.grid_px();
        self.want(key, paths, size)
    }

    /// The cover at `LARGE_PX` (Now Playing); the grid one until it is there.
    pub fn get_large(&mut self, key: &str, paths: &[String]) -> Option<egui::TextureHandle> {
        self.want(key, paths, LARGE_PX)
    }

    /// Make the grid-size covers of `albums` that the covers folder does not
    /// have yet, after everything asked for on screen.
    pub fn prefetch(&mut self, albums: &[Album]) {
        let size = self.grid_px();
        let jobs: VecDeque<Job> = albums
            .iter()
            .filter(|a| self.lookup(&a.key, size).is_none())
            .map(|a| Job {
                key: a.key.clone(),
                size,
                paths: a.tracks.iter().take(CACHE_PATHS).map(|t| t.path.clone()).collect(),
                generation: self.generation,
            })
            .collect();
        let (lock, cv) = &*self.jobs;
        lock.lock().unwrap_or_else(|e| e.into_inner()).back = jobs;
        cv.notify_one();
    }

    /// How many albums the background pass has still to look at (tests).
    #[cfg(test)]
    pub fn prefetching(&self) -> usize {
        self.jobs.0.lock().unwrap_or_else(|e| e.into_inner()).back.len()
    }

    /// The library was read again: keep every texture whose album has the same
    /// first songs (path, size and time) as before, drop the others.
    pub fn library_changed(&mut self, albums: &[Album]) {
        let new: HashMap<String, Vec<(String, i64, i64)>> = albums
            .iter()
            .map(|a| {
                (
                    a.key.clone(),
                    a.tracks.iter().take(CACHE_PATHS).map(|t| (t.path.clone(), t.size, t.mtime_ns)).collect(),
                )
            })
            .collect();
        let gone: Vec<String> =
            self.signatures.iter().filter(|(k, sig)| new.get(*k) != Some(*sig)).map(|(k, _)| k.clone()).collect();
        self.signatures = new;
        if gone.is_empty() {
            return;
        }
        self.generation += 1;
        for k in gone {
            self.tex.remove(&k);
            self.used.retain(|(key, _), _| *key != k);
            self.pending.retain(|(key, _)| *key != k);
            self.dropped.insert(k, self.generation);
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        for _ in 0..PER_FRAME {
            let Ok((key, size, generation, img)) = self.rx.try_recv() else { return };
            if self.dropped.get(&key).is_some_and(|g| generation < *g) {
                continue; // made from songs that have changed since
            }
            self.pending.remove(&(key.clone(), size));
            let tex = img.map(|ci| {
                let options = egui::TextureOptions {
                    mipmap_mode: Some(egui::TextureFilter::Linear),
                    ..egui::TextureOptions::LINEAR
                };
                ctx.load_texture(format!("player cover {key} {size}"), ci, options)
            });
            let entry = self.tex.entry(key.clone()).or_default();
            entry.retain(|(s, _)| *s != size);
            entry.push((size, tex));
            if size > self.grid_px() {
                self.tick += 1;
                self.used.insert((key, size), self.tick);
                self.evict_large();
            }
        }
        ctx.request_repaint(); // more are waiting
    }

    fn evict_large(&mut self) {
        while self.used.len() > MAX_LARGE {
            let Some(oldest) = self.used.iter().min_by_key(|(_, t)| **t).map(|(k, _)| k.clone()) else { return };
            self.used.remove(&oldest);
            if let Some(v) = self.tex.get_mut(&oldest.0) {
                v.retain(|(s, _)| *s != oldest.1);
            }
        }
    }
}

impl Drop for Covers {
    fn drop(&mut self) {
        let (lock, cv) = &*self.jobs;
        let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
        q.stop = true;
        q.front.clear();
        q.back.clear();
        cv.notify_all();
    }
}
