//! Saved scan results (the same SQLite database, schema and row format as
//! `realhires/store.py`, so the Python and the Rust version share them).
//!
//! * Windows: %LOCALAPPDATA%\RealHiRes\results.sqlite
//! * macOS:   ~/Library/Application Support/RealHiRes/results.sqlite
//! * Linux:   $XDG_DATA_HOME/realhires/results.sqlite (default ~/.local/share/realhires)
//!
//! `$REALHIRES_RESULTS` overrides the path.  A saved result is reused when the
//! file has the same size and modification time, was made with the same
//! detection settings and covers as much of the file as asked for.  A moved or
//! renamed file is recognised by size, modification time and a fingerprint of
//! its first and last 64 KB.  Errors are never saved.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use sha1::{Digest, Sha1};

use crate::analysis::{Plot, Spectrogram};
use crate::json::{self, Obj, Style, Value};
use crate::paths::abspath;
use crate::settings::Settings;
use crate::verdict::{FileResult, SavedInfo, Verdict, DETECTOR_VERSION};

pub const SCHEMA_VERSION: i64 = 1;
pub const DB_NAME: &str = "results.sqlite";
pub const DEFAULT_GRAPHS_MB: f64 = 500.0;
const TRIM_EVERY: u64 = 50;
const FP_BYTES: u64 = 64 * 1024;

#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

pub fn data_dir() -> PathBuf {
    let env = |v: &str| std::env::var_os(v).filter(|x| !x.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        env("LOCALAPPDATA").unwrap_or_else(|| crate::settings::home().join("AppData").join("Local")).join("RealHiRes")
    } else if cfg!(target_os = "macos") {
        crate::settings::home().join("Library").join("Application Support").join("RealHiRes")
    } else {
        env("XDG_DATA_HOME").unwrap_or_else(|| crate::settings::home().join(".local").join("share")).join("realhires")
    }
}

pub fn default_path() -> PathBuf {
    std::env::var_os("REALHIRES_RESULTS")
        .filter(|x| !x.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir().join(DB_NAME))
}

/// Fingerprint of the settings that can change a result (the Detection group):
/// the first 16 hex digits of the SHA-1 of `json.dumps(values, sort_keys=True)`,
/// exactly as the Python version computes it.
pub fn settings_key(cfg: &Settings) -> String {
    let mut items: Vec<(String, Value)> = Settings::field_info()
        .iter()
        .filter(|f| f.group == "Detection")
        .map(|f| (f.name.to_string(), cfg.get(f.name).unwrap().to_json()))
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let text = json::dumps(&Value::Object(items), Style::DEFAULT);
    let digest = Sha1::digest(text.as_bytes());
    hex(&digest)[..16].to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// (size, st_mtime_ns) of a file, as Python's `os.stat` gives them.
pub fn stat(path: &str) -> std::io::Result<(i64, i64)> {
    let m = std::fs::metadata(path)?;
    let mtime = m.modified()?;
    let ns = match mtime.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(e) => -(e.duration().as_nanos() as i64),
    };
    Ok((m.len() as i64, ns))
}

/// Hash of the size and the first and last 64 KB.
pub fn fingerprint(path: &str, size: i64) -> std::io::Result<String> {
    let mut h = Sha1::new();
    h.update(size.to_string().as_bytes());
    let mut f = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    (&mut f).take(FP_BYTES).read_to_end(&mut buf)?;
    h.update(&buf);
    if size as u64 > 2 * FP_BYTES {
        f.seek(SeekFrom::Start(size as u64 - FP_BYTES))?;
        buf.clear();
        f.take(FP_BYTES).read_to_end(&mut buf)?;
        h.update(&buf);
    }
    Ok(hex(&h.finalize()))
}

/// Does a result made with `limit_s` (0 = whole file) answer a request for
/// the first `wanted` seconds (None or 0 = the whole file)?
pub fn covers(whole: bool, limit_s: f64, wanted: Option<f64>) -> bool {
    whole || wanted.is_some_and(|w| w != 0.0 && limit_s >= w)
}

pub fn result_to_json(r: &FileResult) -> String {
    json::dumps(&r.to_store_json(), Style::COMPACT)
}

pub fn result_from_json(text: &str, path: &str) -> Result<FileResult, String> {
    let v = json::parse(text).map_err(|e| e.to_string())?;
    FileResult::from_store_json(&v, path).ok_or_else(|| "not a saved result".to_string())
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

fn unzlib(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).ok()?;
    Some(out)
}

fn floats(v: &[f64]) -> Value {
    Value::Array(v.iter().map(|x| Value::Float(*x)).collect())
}

/// (graphs, spectrogram bytes) as the Python version stores them.
pub fn plot_to_blobs(p: &Plot) -> (Vec<u8>, Option<Vec<u8>>) {
    let meta = p.spectrogram.as_ref().map(|sg| {
        Obj::new()
            .put("shape", Value::Array(vec![Value::Int(sg.shape.0 as i64), Value::Int(sg.shape.1 as i64)]))
            .put("db_min", sg.db_min)
            .put("nyquist", sg.nyquist)
            .put("seconds_per_column", sg.seconds_per_column)
            .build()
    });
    let v = Obj::new()
        .put("freqs", floats(&p.freqs))
        .put("db", floats(&p.db))
        .put("nyquist", p.nyquist)
        .put("floor_db", p.floor_db)
        .put("threshold_db", p.threshold_db)
        .put("peak_db", p.peak_db)
        .put("bandwidth_hz", p.bandwidth_hz)
        .put("spectrogram_meta", meta.unwrap_or(Value::Null))
        .build();
    (zlib(json::dumps(&v, Style::DEFAULT).as_bytes()), p.spectrogram.as_ref().map(|sg| sg.data.clone()))
}

pub fn plot_from_blobs(graphs: &[u8], spectro: Option<&[u8]>) -> Option<Plot> {
    let text = unzlib(graphs)?;
    let v = json::parse(&String::from_utf8_lossy(&text)).ok()?;
    let arr = |k: &str| -> Vec<f64> {
        v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default()
    };
    let num = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
    let spectrogram = match (v.get("spectrogram_meta"), spectro) {
        (Some(meta), Some(data)) if !meta.is_null() => {
            let shape = meta.get("shape").and_then(Value::as_array)?;
            Some(Spectrogram {
                data: data.to_vec(),
                shape: (shape.first()?.as_i64()? as usize, shape.get(1)?.as_i64()? as usize),
                db_min: meta.get("db_min").and_then(Value::as_f64)?,
                nyquist: meta.get("nyquist").and_then(Value::as_f64)?,
                seconds_per_column: meta.get("seconds_per_column").and_then(Value::as_f64)?,
            })
        }
        _ => None,
    };
    Some(Plot {
        freqs: arr("freqs"),
        db: arr("db"),
        nyquist: num("nyquist"),
        floor_db: num("floor_db"),
        threshold_db: num("threshold_db"),
        peak_db: num("peak_db"),
        bandwidth_hz: num("bandwidth_hz"),
        spectrogram,
    })
}

const COLUMNS: &str =
    "path, skey, fp, size, mtime_ns, limit_s, whole, detector, app_version, saved, used, result, graphs, spectro, gbytes";

struct Row {
    path: String,
    skey: String,
    fp: String,
    size: i64,
    mtime_ns: i64,
    limit_s: f64,
    whole: bool,
    detector: i64,
    app_version: String,
    saved: f64,
    used: f64,
    result: String,
    graphs: Option<Vec<u8>>,
    spectro: Option<Vec<u8>>,
    gbytes: i64,
}

fn row_from(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok(Row {
        path: r.get(0)?,
        skey: r.get(1)?,
        fp: r.get(2)?,
        size: r.get(3)?,
        mtime_ns: r.get(4)?,
        limit_s: r.get(5)?,
        whole: r.get::<_, i64>(6)? != 0,
        detector: r.get(7)?,
        app_version: r.get(8)?,
        saved: r.get(9)?,
        used: r.get(10)?,
        result: r.get(11)?,
        graphs: r.get(12)?,
        spectro: r.get(13)?,
        gbytes: r.get(14)?,
    })
}

pub struct ResultStore {
    pub path: PathBuf,
    pub graphs_limit: f64,
    graph_writes: u64,
    db: Connection,
}

pub struct StoreStats {
    pub results: i64,
    pub files: i64,
    pub graph_bytes: i64,
    pub disk_bytes: u64,
    pub path: PathBuf,
}

fn is_damage(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::NotADatabase) | Some(rusqlite::ErrorCode::DatabaseCorrupt)
    )
}

impl ResultStore {
    pub fn open(path: Option<&Path>, graphs_mb: f64) -> Result<Self, StoreError> {
        let path = PathBuf::from(abspath(&path.map(Path::to_path_buf).unwrap_or_else(default_path).to_string_lossy()));
        let fail =
            |e: &dyn std::fmt::Display| StoreError(format!("cannot open the saved results in {}: {e}", path.display()));
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| fail(&e))?;
        }
        let db = match Self::connect(&path) {
            Ok(db) => db,
            Err(ConnectError::Store(e)) => return Err(e),
            Err(ConnectError::Sql(e)) if is_damage(&e) => {
                // A damaged file must not stop scanning: keep it aside and start afresh.
                let aside = format!("{}.damaged-{}", path.display(), now() as i64);
                std::fs::rename(&path, aside).map_err(|e| fail(&e))?;
                Self::connect(&path).map_err(|e| match e {
                    ConnectError::Store(s) => s,
                    ConnectError::Sql(e) => fail(&e),
                })?
            }
            Err(ConnectError::Sql(e)) => return Err(fail(&e)),
        };
        Ok(ResultStore { path, graphs_limit: graphs_mb.max(0.0) * 1e6, graph_writes: 0, db })
    }

    fn connect(path: &Path) -> Result<Connection, ConnectError> {
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(30))?;
        db.execute_batch("PRAGMA auto_vacuum=INCREMENTAL")?;
        let _ = db.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0));
        db.execute_batch("PRAGMA synchronous=NORMAL")?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT)")?;
        let schema: Option<String> =
            db.query_row("SELECT value FROM meta WHERE key='schema'", [], |r| r.get(0)).optional()?;
        if schema.and_then(|s| s.parse::<i64>().ok()).is_some_and(|s| s > SCHEMA_VERSION) {
            return Err(ConnectError::Store(StoreError(format!(
                "{} was written by a newer Real-Hi-Res; it is left untouched",
                path.display()
            ))));
        }
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS results (
            path TEXT NOT NULL, skey TEXT NOT NULL, fp TEXT NOT NULL,
            size INTEGER NOT NULL, mtime_ns INTEGER NOT NULL,
            limit_s REAL NOT NULL, whole INTEGER NOT NULL,
            detector INTEGER NOT NULL, app_version TEXT NOT NULL,
            saved REAL NOT NULL, used REAL NOT NULL,
            result TEXT NOT NULL, graphs BLOB, spectro BLOB, gbytes INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (path, skey));
            CREATE INDEX IF NOT EXISTS results_moved ON results (size, mtime_ns);",
        )?;
        db.execute("INSERT OR REPLACE INTO meta VALUES ('schema', ?1)", params![SCHEMA_VERSION.to_string()])?;
        db.query_row("SELECT count(*) FROM results", [], |r| r.get::<_, i64>(0))?; // fails here on a damaged file
        Ok(db)
    }

    /// The saved result for this file as it is now, or None.
    pub fn lookup(&self, path: &str, skey: &str, wanted: Option<f64>) -> Option<FileResult> {
        let path = abspath(path);
        let (size, mtime) = stat(&path).ok()?;
        match self.lookup_inner(&path, skey, wanted, size, mtime) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Real-Hi-Res: ignoring a saved result for {path}: {e}");
                None
            }
        }
    }

    fn lookup_inner(
        &self,
        path: &str,
        skey: &str,
        wanted: Option<f64>,
        size: i64,
        mtime: i64,
    ) -> Result<Option<FileResult>, Box<dyn std::error::Error>> {
        let mut row = self
            .db
            .query_row(
                &format!("SELECT {COLUMNS} FROM results WHERE path=?1 AND skey=?2"),
                params![path, skey],
                row_from,
            )
            .optional()?;
        let mut moved_from = None;
        if !row.as_ref().is_some_and(|r| r.size == size && r.mtime_ns == mtime && covers(r.whole, r.limit_s, wanted)) {
            row = None;
            let mut stmt = self.db.prepare(&format!(
                "SELECT {COLUMNS} FROM results WHERE size=?1 AND mtime_ns=?2 AND skey=?3 AND path!=?4 ORDER BY saved DESC"
            ))?;
            let cands: Vec<Row> =
                stmt.query_map(params![size, mtime, skey, path], row_from)?.collect::<Result<_, _>>()?;
            let mut fp: Option<String> = None;
            for cand in cands {
                if !covers(cand.whole, cand.limit_s, wanted) {
                    continue;
                }
                if fp.is_none() {
                    fp = Some(fingerprint(path, size)?);
                }
                if Some(&cand.fp) == fp.as_ref() {
                    moved_from = Some(cand.path.clone());
                    row = Some(cand);
                    break;
                }
            }
        }
        let Some(row) = row else {
            return Ok(None);
        };
        let mut r = result_from_json(&row.result, path)?;
        if moved_from.is_some() {
            // remember it under its new name too
            self.db.execute(
                &format!("INSERT OR REPLACE INTO results ({COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)"),
                params![
                    path, row.skey, row.fp, row.size, row.mtime_ns, row.limit_s, row.whole as i64, row.detector,
                    row.app_version, row.saved, row.used, row.result, row.graphs, row.spectro, row.gbytes
                ],
            )?;
        }
        r.saved = Some(SavedInfo {
            time: row.saved,
            version: row.app_version.clone(),
            detector: row.detector,
            outdated: row.detector < DETECTOR_VERSION,
            graphs: row.graphs.is_some(),
            moved_from,
        });
        Ok(Some(r))
    }

    pub fn get_plot(&self, path: &str, skey: &str) -> Option<Plot> {
        let path = abspath(path);
        let res: rusqlite::Result<Option<(Option<Vec<u8>>, Option<Vec<u8>>)>> = self
            .db
            .query_row("SELECT graphs, spectro FROM results WHERE path=?1 AND skey=?2", params![path, skey], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional();
        match res {
            Ok(Some((Some(g), s))) => {
                let _ =
                    self.db.execute("UPDATE results SET used=?1 WHERE path=?2 AND skey=?3", params![now(), path, skey]);
                let p = plot_from_blobs(&g, s.as_deref());
                if p.is_none() {
                    eprintln!("Real-Hi-Res: cannot read saved graphs for {path}");
                }
                p
            }
            Ok(_) => None,
            Err(e) => {
                eprintln!("Real-Hi-Res: cannot read saved graphs for {path}: {e}");
                None
            }
        }
    }

    /// Save one result (and its graphs, if given).  False when not saved.
    pub fn put(&mut self, r: &FileResult, skey: &str, wanted: Option<f64>, plot: Option<&Plot>) -> bool {
        if r.verdict == Verdict::Error {
            return false;
        }
        let path = abspath(&r.path);
        let Ok((size, mtime)) = stat(&path) else {
            return false;
        };
        let Ok(fp) = fingerprint(&path, size) else {
            return false;
        };
        let wanted = wanted.filter(|w| *w != 0.0);
        let whole = wanted.is_none() || r.duration.is_some_and(|d| d != 0.0 && r.analysed_seconds >= d - 0.5);
        let t = now();
        let (mut graphs, mut spectro) = (None, None);
        if let Some(p) = plot.filter(|_| self.graphs_limit > 0.0) {
            let (g, s) = plot_to_blobs(p);
            graphs = Some(g);
            spectro = s;
        }
        let res = (|| -> rusqlite::Result<()> {
            self.db.execute_batch("BEGIN IMMEDIATE")?;
            if graphs.is_none() {
                // keep graphs saved earlier for this very file
                let old: Option<(Option<Vec<u8>>, Option<Vec<u8>>)> = self
                    .db
                    .query_row(
                        "SELECT graphs, spectro FROM results WHERE path=?1 AND skey=?2 AND size=?3 AND mtime_ns=?4",
                        params![path, skey, size, mtime],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                if let Some((g, s)) = old {
                    graphs = g;
                    spectro = s;
                }
            }
            // the file changed: results saved under other settings are stale too
            self.db.execute(
                "DELETE FROM results WHERE path=?1 AND (size!=?2 OR mtime_ns!=?3)",
                params![path, size, mtime],
            )?;
            let gbytes = graphs.as_ref().map_or(0, Vec::len) + spectro.as_ref().map_or(0, Vec::len);
            self.db.execute(
                &format!("INSERT OR REPLACE INTO results ({COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)"),
                params![
                    path,
                    skey,
                    fp,
                    size,
                    mtime,
                    wanted.unwrap_or(0.0),
                    whole as i64,
                    DETECTOR_VERSION,
                    crate::VERSION,
                    t,
                    t,
                    result_to_json(r),
                    graphs,
                    spectro,
                    gbytes as i64
                ],
            )?;
            self.db.execute_batch("COMMIT")
        })();
        if let Err(e) = res {
            self.rollback();
            eprintln!("Real-Hi-Res: cannot save the result for {path}: {e}");
            return false;
        }
        if plot.is_some() {
            self.graph_writes += 1;
            if self.graph_writes % TRIM_EVERY == 0 {
                self.trim_graphs();
            }
        }
        true
    }

    /// Add graphs to a saved result (they were drawn later, on demand).
    pub fn put_plot(&self, path: &str, skey: &str, plot: &Plot) {
        if self.graphs_limit <= 0.0 {
            return;
        }
        let (g, s) = plot_to_blobs(plot);
        let n = g.len() + s.as_ref().map_or(0, Vec::len);
        if let Err(e) = self.db.execute(
            "UPDATE results SET graphs=?1, spectro=?2, gbytes=?3, used=?4 WHERE path=?5 AND skey=?6",
            params![g, s, n as i64, now(), abspath(path), skey],
        ) {
            eprintln!("Real-Hi-Res: cannot save graphs for {path}: {e}");
        }
    }

    /// The file's tags changed but its audio did not (lyrics were written into
    /// it, 2.0): move its saved results to its new size and time, so it is not
    /// analysed again (Python `ResultStore.retag`).  Returns the rows moved.
    pub fn retag(&self, path: &str, old_size: i64, old_mtime_ns: i64) -> usize {
        let path = abspath(path);
        let Ok((size, mtime)) = stat(&path) else { return 0 };
        let Ok(fp) = fingerprint(&path, size) else { return 0 };
        self.db
            .execute(
                "UPDATE results SET size=?1, mtime_ns=?2, fp=?3 WHERE path=?4 AND size=?5 AND mtime_ns=?6",
                params![size, mtime, fp, path, old_size, old_mtime_ns],
            )
            .unwrap_or(0)
    }

    /// Drop the least recently viewed graphs above the size limit.
    pub fn trim_graphs(&self) {
        let res = (|| -> rusqlite::Result<()> {
            let mut total: i64 = self.db.query_row("SELECT COALESCE(SUM(gbytes), 0) FROM results", [], |r| r.get(0))?;
            if total as f64 <= self.graphs_limit {
                return Ok(());
            }
            let target = 0.9 * self.graphs_limit;
            let mut drop = vec![];
            {
                let mut stmt = self.db.prepare("SELECT rowid, gbytes FROM results WHERE gbytes > 0 ORDER BY used")?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
                for row in rows {
                    let (id, n) = row?;
                    if total as f64 <= target {
                        break;
                    }
                    drop.push(id);
                    total -= n;
                }
            }
            self.db.execute_batch("BEGIN IMMEDIATE")?;
            for id in drop {
                self.db
                    .execute("UPDATE results SET graphs=NULL, spectro=NULL, gbytes=0 WHERE rowid=?1", params![id])?;
            }
            self.db.execute_batch("COMMIT")?;
            self.db.execute_batch("PRAGMA incremental_vacuum")
        })();
        if let Err(e) = res {
            self.rollback();
            eprintln!("Real-Hi-Res: cannot trim saved graphs: {e}");
        }
    }

    /// Delete the saved results of these files (all of them when None).
    pub fn forget(&self, paths: Option<&[String]>) -> Result<usize, StoreError> {
        let res = (|| -> rusqlite::Result<usize> {
            match paths {
                None => {
                    let n: i64 = self.db.query_row("SELECT count(*) FROM results", [], |r| r.get(0))?;
                    self.db.execute_batch("DELETE FROM results; VACUUM;")?;
                    Ok(n as usize)
                }
                Some(ps) => {
                    self.db.execute_batch("BEGIN IMMEDIATE")?;
                    let mut n = 0;
                    for p in ps {
                        n += self.db.execute("DELETE FROM results WHERE path=?1", params![abspath(p)])?;
                    }
                    self.db.execute_batch("COMMIT")?;
                    Ok(n)
                }
            }
        })();
        res.map_err(|e| {
            self.rollback();
            StoreError(e.to_string())
        })
    }

    pub fn stats(&self) -> StoreStats {
        let (results, graph_bytes, files) = self
            .db
            .query_row("SELECT count(*), COALESCE(SUM(gbytes), 0), COUNT(DISTINCT path) FROM results", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap_or((0, 0, 0));
        let disk = ["", "-wal"]
            .iter()
            .filter_map(|ext| std::fs::metadata(format!("{}{ext}", self.path.display())).ok())
            .map(|m| m.len())
            .sum();
        StoreStats { results, files, graph_bytes, disk_bytes: disk, path: self.path.clone() }
    }

    fn rollback(&self) {
        let _ = self.db.execute_batch("ROLLBACK");
    }
}

impl Drop for ResultStore {
    fn drop(&mut self) {
        self.trim_graphs();
    }
}

enum ConnectError {
    Sql(rusqlite::Error),
    Store(StoreError),
}

impl From<rusqlite::Error> for ConnectError {
    fn from(e: rusqlite::Error) -> Self {
        ConnectError::Sql(e)
    }
}

/// The store as the settings ask for it, or None when saving results is
/// turned off or the database cannot be used (scanning works without it).
pub fn open_store(cfg: &Settings) -> Option<ResultStore> {
    if !cfg.save_results {
        return None;
    }
    match ResultStore::open(None, cfg.saved_graphs_mb as f64) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("Real-Hi-Res: {e}; results are not saved this time.");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_key_matches_python() {
        // python3 -c "from realhires.store import settings_key; print(settings_key())"
        assert_eq!(settings_key(&Settings::default()), PYTHON_DEFAULT_SKEY);
    }

    const PYTHON_DEFAULT_SKEY: &str = include_str!("../tests/python_default_skey.txt");

    #[test]
    fn saves_and_finds_moved_files() {
        let dir = std::env::temp_dir().join(format!("rhr-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.flac");
        std::fs::write(&a, vec![7u8; 300_000]).unwrap();
        let mut st = ResultStore::open(Some(&dir.join("r.sqlite")), 500.0).unwrap();
        let mut r = FileResult::new(&a.to_string_lossy(), Verdict::Fake);
        r.summary = "x".into();
        r.cliffs_db = vec![(44100, 30.5)];
        r.upsample_edge_hz = Some(f64::NAN);
        assert!(st.put(&r, "k", None, None));
        let got = st.lookup(&a.to_string_lossy(), "k", None).unwrap();
        assert_eq!(got.cliffs_db, vec![(44100, 30.5)]);
        assert!(got.upsample_edge_hz.unwrap().is_nan());
        let b = dir.join("b.flac");
        std::fs::rename(&a, &b).unwrap();
        let moved = st.lookup(&b.to_string_lossy(), "k", None).unwrap();
        assert!(moved.saved.unwrap().moved_from.is_some());
        assert!(st.lookup(&b.to_string_lossy(), "other", None).is_none());
        drop(st);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
